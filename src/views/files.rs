//! Files tab: the inbox and live transfer progress, both directions, like
//! the Android Files tab.

use gpui_kit::component::button::Button;
use gpui_kit::component::label::Label;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::gpui::prelude::FluentBuilder as _;
use gpui_kit::gpui::{
    div, px, relative, rgb, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_kit_assets::IconName;

use crate::paths;
use crate::state::{Session, TransferState};

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

pub struct FilesView {
    session: Entity<Session>,
}

impl FilesView {
    pub fn new(session: Entity<Session>, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self { session }
    }
}

impl Render for FilesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let theme = cx.theme().clone();
        let inbox = paths::inbox_dir();

        let mut list = div()
            .id("files-list")
            .flex()
            .flex_col()
            .size_full()
            .overflow_y_scroll()
            .child(
                // Toolbar, not a headline: the inbox path is reference
                // material, muted under the action.
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(Label::new("Inbox").flex_shrink_0())
                    .child(
                        Label::new(inbox.display().to_string())
                            .text_sm()
                            .truncate()
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        Button::new("open-inbox")
                            .icon(IconName::FolderOpen)
                            .label("Open folder")
                            .compact()
                            .ml_auto()
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.spawn(async move |_, cx| {
                                    let inbox = paths::inbox_dir();
                                    cx.background_executor()
                                        .spawn(async move {
                                            let _ = std::process::Command::new("xdg-open")
                                                .arg(inbox)
                                                .spawn();
                                        })
                                        .await;
                                })
                                .detach();
                            })),
                    ),
            );

        if session.transfers.is_empty() {
            list = list.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(
                        Icon::new(IconName::FolderOpen)
                            .with_size(px(28.))
                            .text_color(theme.muted_foreground),
                    )
                    .child(Label::new("No transfers yet"))
                    .child(
                        Label::new(
                            "Files you receive land in the inbox automatically. \
                             Send one from a device row on the Devices page.",
                        )
                        .text_sm()
                        .text_color(theme.muted_foreground),
                    ),
            );
        }

        for t in session.transfers.clone() {
            let name = t
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "file".into());
            let pct = if t.total > 0 { t.sent * 100 / t.total } else { 0 };
            let state = match t.state {
                TransferState::Active => format!("{pct}%"),
                TransferState::Done => "done".to_string(),
                TransferState::Failed => "failed".to_string(),
            };
            // Peer as the display name from the roster; the npub is not
            // human information here.
            let peer = session
                .roster
                .iter()
                .find(|d| d.npub == t.peer)
                .map(|d| d.display_name())
                .unwrap_or_else(|| "device".to_string());
            let direction = if t.outbound { "to" } else { "from" };
            // Human sizes: "3.4 MB of 12.0 MB" beats a bare percentage.
            let size_text = if t.total > 0 {
                format!("{} of {}", human_size(t.sent), human_size(t.total))
            } else if t.outbound {
                human_size(t.sent)
            } else {
                String::new()
            };
            let row = div()
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
                .child(
                    div()
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
                                .child(Label::new(name).truncate())
                                .child(
                                    Label::new(state).text_sm().flex_shrink_0().text_color(
                                        match t.state {
                                            TransferState::Failed => {
                                                gpui_kit::gpui::Hsla::from(rgb(0xf87171))
                                            }
                                            TransferState::Done => {
                                                gpui_kit::gpui::Hsla::from(rgb(0x22c55e))
                                            }
                                            TransferState::Active => theme.muted_foreground,
                                        },
                                    ),
                                ),
                        )
                        .child(
                            Label::new(format!("{direction} {peer}"))
                                .text_sm()
                                .text_color(theme.muted_foreground),
                        )
                        .when(!size_text.is_empty(), |s| {
                            s.child(
                                Label::new(size_text)
                                    .text_xs()
                                    .text_color(theme.muted_foreground),
                            )
                        })
                        // Live progress bar for active transfers; a
                        // percentage alone reads as idle text.
                        .when(t.state == TransferState::Active && t.total > 0, |s| {
                            s.child(
                                div()
                                    .rounded_full()
                                    .h_1()
                                    .w_full()
                                    .bg(theme.border)
                                    .overflow_hidden()
                                    .child(
                                        div()
                                            .rounded_full()
                                            .h_1()
                                            .w(relative(pct as f32 / 100.0))
                                            .bg(theme.primary),
                                    ),
                            )
                        }),
                );
            list = list.child(row);
        }

        list.into_any_element()
    }
}
