//! Files tab, like the Android Files tab: Received (the inbox folder, with
//! who sent each file) and Sent (the send history), each with its live
//! transfers on top.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::label::Label;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::gpui::prelude::FluentBuilder as _;
use gpui_kit::gpui::{
    div, px, rgb, AnyElement, Context, Entity, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_kit_assets::IconName;

use crate::paths;
use crate::state::{Session, Transfer, TransferState};

/// Bytes as a human reads them: "856 B", "3.4 MB", "1.2 GB".
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// "14:05" today, "Nov 24 14:05" before.
fn when(at_ms: u64) -> String {
    let Ok(ts) = jiff::Timestamp::from_millisecond(at_ms as i64) else {
        return String::new();
    };
    let tz = jiff::tz::TimeZone::system();
    let at = ts.to_zoned_datetime(tz.clone());
    let today = jiff::Timestamp::now().to_zoned_datetime(tz).date();
    if at.date() == today {
        at.strftime("%H:%M").to_string()
    } else {
        at.strftime("%b %-d %H:%M").to_string()
    }
}

/// Run a program on a path without blocking the UI.
fn launch(cx: &mut Context<FilesView>, program: &'static str, arg: PathBuf) {
    cx.spawn(async move |_, cx| {
        cx.background_executor()
            .spawn(async move {
                let _ = std::process::Command::new(program).arg(arg).spawn();
            })
            .await;
    })
    .detach();
}

/// A file in the inbox folder.
#[derive(Clone)]
struct InboxFile {
    path: PathBuf,
    size: u64,
    at_ms: u64,
}

/// The inbox's finished files, newest first. Partials and the resume
/// sidecars are not files yet.
fn list_inbox(dir: &Path) -> Vec<InboxFile> {
    let mut files: Vec<InboxFile> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name.ends_with(".noscat-part") || name.ends_with(".noscat-sha") {
                return None;
            }
            let meta = e.metadata().ok().filter(|m| m.is_file())?;
            let at_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            Some(InboxFile { path: e.path(), size: meta.len(), at_ms })
        })
        .collect();
    files.sort_by(|a, b| b.at_ms.cmp(&a.at_ms));
    files
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Received,
    Sent,
}

pub struct FilesView {
    session: Entity<Session>,
    side: Side,
    /// The inbox listing, re-read at most once a second and whenever a
    /// receive finishes or a file is deleted here.
    inbox: Vec<InboxFile>,
    listed_at: Option<Instant>,
    listed_done: usize,
    /// The inbox file waiting for its delete confirmation.
    confirm_delete: Option<PathBuf>,
}

impl FilesView {
    pub fn new(session: Entity<Session>, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            session,
            side: Side::Received,
            inbox: Vec::new(),
            listed_at: None,
            listed_done: 0,
            confirm_delete: None,
        }
    }

    fn relist(&mut self, done: usize) {
        let stale = self.listed_at.is_none_or(|t| t.elapsed() > Duration::from_secs(1));
        if stale || done != self.listed_done {
            self.inbox = list_inbox(&paths::inbox_dir());
            self.listed_at = Some(Instant::now());
            self.listed_done = done;
        }
    }
}

/// The state word and its colour for a transfer row.
fn state_label(t: &Transfer, muted: Hsla) -> (String, Hsla) {
    let pct = if t.total > 0 { t.sent * 100 / t.total } else { 0 };
    match t.state {
        TransferState::Active if t.sent == 0 => ("connecting".into(), muted),
        TransferState::Active => (format!("{pct}%"), muted),
        TransferState::Done if t.outbound => ("delivered".into(), rgb(0x22c55e).into()),
        TransferState::Done => ("received".into(), rgb(0x22c55e).into()),
        // Incomplete is not a failure: sending the same file again resumes.
        TransferState::Partial => ("partial, will resume".into(), rgb(0xeab308).into()),
        TransferState::Cancelled => ("stopped".into(), rgb(0xeab308).into()),
        TransferState::Failed => ("failed".into(), rgb(0xf87171).into()),
    }
}

impl Render for FilesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let theme = cx.theme().clone();
        let muted = theme.muted_foreground;
        let transfers = session.transfers.clone();
        let name_of = |npub: &str| session.peer_name(npub);
        let done = transfers.iter().filter(|t| !t.outbound && t.state == TransferState::Done).count();
        let names: Vec<String> = transfers.iter().map(|t| name_of(&t.peer)).collect();
        let _ = session;
        self.relist(done);

        let me = cx.entity().downgrade();
        let tabs = TabBar::new("files-side")
            .underline()
            .selected_index(if self.side == Side::Received { 0 } else { 1 })
            .child(Tab::new().label("Received"))
            .child(Tab::new().label("Sent"))
            .on_click(move |i, _, cx| {
                let side = if *i == 0 { Side::Received } else { Side::Sent };
                let _ = me.update(cx, |this, cx| {
                    this.side = side;
                    this.confirm_delete = None;
                    cx.notify();
                });
            });

        // Toolbar under the switch, Received only: the inbox folder.
        let toolbar = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(theme.border);
        let toolbar = match self.side {
            Side::Received => toolbar
                .child(
                    Label::new(paths::inbox_dir().display().to_string())
                        .text_sm()
                        .truncate()
                        .text_color(muted),
                )
                .child(
                    Button::new("open-inbox")
                        .icon(IconName::FolderOpen)
                        .label("Open folder")
                        .compact()
                        .ml_auto()
                        .on_click(cx.listener(|_, _, _, cx| launch(cx, "xdg-open", paths::inbox_dir()))),
                ),
            Side::Sent => toolbar,
        };

        let has_history = transfers.iter().any(|t| t.outbound && t.state != TransferState::Active);
        let mut list = div().id("files-list").flex().flex_col().flex_1().min_h_0().overflow_y_scroll();
        let mut rows = 0;

        // Live transfers of this side first, newest on top.
        let outbound = self.side == Side::Sent;
        for (index, t) in transfers.iter().enumerate().rev() {
            if t.outbound != outbound {
                continue;
            }
            let live_receive = !t.outbound && t.live && t.state != TransferState::Done;
            if !t.outbound && !live_receive {
                // Finished receives show as inbox files below.
                continue;
            }
            rows += 1;
            list = list.child(self.transfer_row(index, t, &names[index], cx));
        }

        if self.side == Side::Received {
            for f in self.inbox.clone() {
                // The sender, when this app received it.
                let from = transfers
                    .iter()
                    .zip(&names)
                    .rev()
                    .find(|(t, _)| !t.outbound && t.state == TransferState::Done && t.path.as_deref() == Some(f.path.as_path()))
                    .map(|(_, n)| n.clone());
                rows += 1;
                list = list.child(self.inbox_row(rows, f, from, cx));
            }
        }

        if rows == 0 {
            let (title, hint) = match self.side {
                Side::Received => ("Nothing received yet", "Files members send you land in the inbox folder."),
                Side::Sent => ("Nothing sent yet", "Send a file from a device row on the Devices page."),
            };
            list = list.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .py_16()
                    .child(Icon::new(IconName::FolderOpen).with_size(px(28.)).text_color(muted))
                    .child(Label::new(title))
                    .child(Label::new(hint).text_sm().text_color(muted)),
            );
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            // The switch, with Clear history right-aligned on its row
            // while Sent is showing.
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .px_4()
                    .pt_2()
                    .child(tabs)
                    .when(self.side == Side::Sent, |row| {
                        row.child(
                            Button::new("clear-sent")
                                .label("Clear history")
                                .compact()
                                .ml_auto()
                                .disabled(!has_history)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.session.update(cx, |s, cx| s.clear_sent(cx));
                                })),
                        )
                    }),
            )
            .when(self.side == Side::Received, |d| d.child(toolbar))
            .child(list)
    }
}

impl FilesView {
    /// A row's frame: file icon, then the given body.
    fn row(&self, body: impl IntoElement, cx: &Context<Self>) -> gpui_kit::gpui::Div {
        let theme = cx.theme();
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .px_4()
            .py_3()
            .border_b_1()
            .border_color(theme.border)
            .child(
                Icon::new(IconName::FileText)
                    .with_size(px(20.))
                    .flex_shrink_0()
                    .text_color(theme.muted_foreground),
            )
            .child(body)
    }

    /// A send (any state) or a live receive, with its progress bar while
    /// active.
    fn transfer_row(&self, index: usize, t: &Transfer, peer: &str, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let muted = theme.muted_foreground;
        let (state, color) = state_label(t, muted);
        let direction = if t.outbound { "to" } else { "from" };
        let active = t.state == TransferState::Active;
        let size = if active && t.total > 0 {
            format!("{} of {}", human_size(t.sent), human_size(t.total))
        } else {
            human_size(t.total)
        };
        let detail = format!("{direction} {peer} · {size} · {}", when(t.at_ms));
        let id = t.id;
        let resendable = t.outbound && !active && t.path.as_ref().is_some_and(|p| p.exists());

        let body = div()
            .flex()
            .flex_col()
            .gap_1()
            .min_w_0()
            .flex_1()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(Label::new(t.name.clone()).truncate())
                    .child(Label::new(state).text_sm().flex_shrink_0().text_color(color)),
            )
            .child(Label::new(detail).text_sm().truncate().text_color(muted))
            // Determinate once bytes move; a sweeping bar while it connects.
            .when(active, |s| {
                let pct = if t.total > 0 { t.sent as f32 * 100.0 / t.total as f32 } else { 0.0 };
                s.child(
                    Progress::new(SharedString::from(format!("progress-{index}")))
                        .value(pct)
                        .loading(t.sent == 0),
                )
            });

        let actions = div()
            .flex()
            .flex_row()
            .gap_2()
            .flex_shrink_0()
            .when(active && t.outbound, |s| {
                s.child(
                    Button::new(SharedString::from(format!("stop-{index}")))
                        .label("Stop")
                        .compact()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.session.update(cx, |s, cx| s.cancel_send(id, cx));
                        })),
                )
            })
            .when(resendable && t.state != TransferState::Done, |s| {
                s.child(
                    Button::new(SharedString::from(format!("resend-{index}")))
                        .label("Send again")
                        .compact()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.session.update(cx, |s, cx| s.resend(index, cx));
                        })),
                )
            });

        self.row(div().flex().flex_row().items_center().gap_3().flex_1().min_w_0().child(body).child(actions), cx)
            .into_any_element()
    }

    /// A file in the inbox: open it, or delete it (asks first).
    fn inbox_row(&self, n: usize, f: InboxFile, from: Option<String>, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let muted = theme.muted_foreground;
        let name = f.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut detail = Vec::new();
        if let Some(from) = from {
            detail.push(format!("from {from}"));
        }
        detail.push(human_size(f.size));
        detail.push(when(f.at_ms));
        let confirming = self.confirm_delete.as_deref() == Some(f.path.as_path());
        let open_path = f.path.clone();
        let delete_path = f.path.clone();

        let body = div()
            .flex()
            .flex_col()
            .gap_1()
            .min_w_0()
            .flex_1()
            .child(Label::new(name).truncate())
            .child(Label::new(detail.join(" · ")).text_sm().truncate().text_color(muted));

        let actions = div()
            .flex()
            .flex_row()
            .gap_2()
            .flex_shrink_0()
            .child(
                Button::new(SharedString::from(format!("open-{n}")))
                    .label("Open")
                    .compact()
                    .on_click(cx.listener(move |_, _, _, cx| launch(cx, "xdg-open", open_path.clone()))),
            )
            .child(if confirming {
                Button::new(SharedString::from(format!("delete-{n}")))
                    .label("Delete file?")
                    .danger()
                    .compact()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let _ = std::fs::remove_file(&delete_path);
                        this.confirm_delete = None;
                        this.listed_at = None;
                        cx.notify();
                    }))
            } else {
                Button::new(SharedString::from(format!("delete-{n}")))
                    .label("Delete")
                    .compact()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.confirm_delete = Some(delete_path.clone());
                        cx.notify();
                    }))
            });

        self.row(div().flex().flex_row().items_center().gap_3().flex_1().min_w_0().child(body).child(actions), cx)
            .into_any_element()
    }
}
