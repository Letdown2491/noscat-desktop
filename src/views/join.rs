//! Join flow: the desktop counterpart of the Android app's QR join. The
//! desktop has no camera, so the flow is paste: paste a mesh member's
//! pairing code, which sends that member a join request, then wait while
//! the member approves it.
//!
//! The screen walks the steps in the order they happen: paste the code,
//! then a waiting card (countdown, what to do on the other device, and this
//! device's ID to compare), then the node starts by itself. Joining from a
//! CLI member (`noscat mesh invite`) needs this device's ID, which sits in
//! a collapsed section. A device that already belongs to a mesh sees a
//! Reconnect screen instead.

use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::gpui::prelude::FluentBuilder as _;
use gpui_kit::gpui::{
    div, img, px, AppContext as _, ClipboardItem, Context, Div, Entity,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, RenderImage,
    StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_kit_assets::IconName;

use crate::mesh as mesh_helpers;
use crate::paths;
use crate::state::Session;

/// How long a join request waits for the member's approval.
const JOIN_TIMEOUT_SECS: u64 = 120;

/// How far through the join flow we are; drives which card shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    /// The join request is out; the member has until the deadline.
    Waiting { since: Instant },
    /// The wait ran out without an approval: not an error, the member may
    /// just not have seen the request yet.
    TimedOut,
}

/// Render `text` as a QR code image (white background, quiet zone included).
/// The device ID as a scannable QR. Shared by the join screen and the
/// Settings device ID card.
pub fn qr_image(text: &str) -> Option<std::sync::Arc<RenderImage>> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    let colors = code.to_colors();
    let modules = code.width();
    let quiet: u32 = 4;
    let scale: u32 = 8;
    let dim = (modules as u32 + quiet * 2) * scale;
    let mut buf = image::RgbaImage::from_pixel(dim, dim, image::Rgba([255, 255, 255, 255]));
    for (y, row) in colors.chunks(modules).enumerate() {
        for (x, color) in row.iter().enumerate() {
            if *color == qrcode::Color::Dark {
                for dy in 0..scale {
                    for dx in 0..scale {
                        buf.put_pixel(
                            (x as u32 + quiet) * scale + dx,
                            (y as u32 + quiet) * scale + dy,
                            image::Rgba([0, 0, 0, 255]),
                        );
                    }
                }
            }
        }
    }
    Some(std::sync::Arc::new(RenderImage::new([image::Frame::new(buf)])))
}

/// Normalize what the user pasted into the join target: a pairing code that
/// names relays (a private mesh's) stays whole, since the join goes through
/// those relays; any other pairing code becomes the member's bare ID.
/// `None` when it is not a pairing code at all.
fn join_target(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let pk = noscat_client::parse_pairing_target(raw).ok()?;
    if !noscat_client::parse_pairing_relays(raw).is_empty() {
        return Some(raw.to_string());
    }
    use nostr::nips::nip19::ToBech32 as _;
    Some(pk.to_bech32().unwrap_or_else(|_| raw.to_string()))
}

pub struct JoinView {
    session: Entity<Session>,
    code_input: Entity<InputState>,
    phase: Phase,
    /// A real failure (not a timeout), shown under the form.
    error: Option<String>,
    /// The "Joining from a CLI device?" section is open.
    show_cli: bool,
    /// The device ID was copied to the clipboard.
    copied: bool,
}

impl JoinView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let code_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Paste the pairing code here")
        });
        // Re-render as the code is typed (the button enables once it
        // parses), and Enter submits.
        cx.subscribe_in(&code_input, window, |this, _, ev: &InputEvent, _, cx| match ev {
            InputEvent::Change => {
                this.error = None;
                cx.notify();
            }
            InputEvent::PressEnter { .. } => this.request_join(cx),
            _ => {}
        })
        .detach();
        Self {
            session,
            code_input,
            phase: Phase::Idle,
            error: None,
            show_cli: false,
            copied: false,
        }
    }

    /// Send the join request to the pasted member and wait for its
    /// approval in the background; on success the node starts and the
    /// root view flips to the tabs.
    fn request_join(&mut self, cx: &mut Context<Self>) {
        if matches!(self.phase, Phase::Waiting { .. }) {
            return;
        }
        let Some(owner) = join_target(&self.code_input.read(cx).value()) else {
            self.error = Some(
                "That is not a pairing code. Copy it from Settings > Device ID on a \
                 device in the mesh."
                    .into(),
            );
            cx.notify();
            return;
        };
        self.error = None;
        self.phase = Phase::Waiting { since: Instant::now() };
        cx.notify();

        // Tick once a second for the countdown while waiting.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let waiting = this
                    .update(cx, |t, cx| {
                        cx.notify();
                        matches!(t.phase, Phase::Waiting { .. })
                    })
                    .unwrap_or(false);
                if !waiting {
                    break;
                }
            }
        })
        .detach();

        let session = self.session.clone();
        let dir = paths::mesh_dir();
        cx.spawn(async move |this, cx| {
            let joined = cx.background_executor().spawn(async move {
                mesh_helpers::prepare(&dir, Some(&owner))?;
                mesh_helpers::join(&dir, JOIN_TIMEOUT_SECS, &owner)
            });
            let outcome = joined.await;
            let _ = this.update(cx, |t, cx| {
                match outcome {
                    Ok(_) => {
                        t.phase = Phase::Idle;
                        session.update(cx, |s, cx| s.start_node(cx));
                    }
                    Err(e) if e.to_string().contains("no invitation arrived in time") => {
                        t.phase = Phase::TimedOut;
                    }
                    Err(e) => {
                        t.phase = Phase::Idle;
                        t.error = Some(format!("Could not join: {e}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// A numbered step's card frame.
fn card(cx: &Context<JoinView>) -> Div {
    let theme = cx.theme();
    div()
        .flex()
        .flex_col()
        .gap_3()
        .w_full()
        .p_5()
        .border_1()
        .border_color(theme.border)
        .rounded(px(10.))
        .bg(theme.secondary)
}

fn step_title(n: usize, text: &str, cx: &Context<JoinView>) -> Div {
    let theme = cx.theme();
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .flex_shrink_0()
                .size(px(22.))
                .rounded_full()
                .bg(theme.primary)
                .child(Label::new(n.to_string()).text_xs().text_color(theme.primary_foreground)),
        )
        .child(Label::new(text.to_string()).text_size(px(15.)))
}

impl JoinView {
    /// Step 1: paste the code (the form), or what was sent while waiting.
    fn render_paste(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let valid = join_target(&self.code_input.read(cx).value()).is_some();
        let waiting = matches!(self.phase, Phase::Waiting { .. });
        card(cx)
            .child(step_title(1, "Paste a pairing code from a device in the mesh", cx))
            .child(
                Label::new(
                    "On that device, open Settings > Device ID and press Copy. On a \
                     computer running the Noscat CLI, run noscat mesh qr.",
                )
                .text_sm()
                .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(Input::new(&self.code_input).disabled(waiting)))
                    .child(
                        Button::new("join")
                            .label("Request to join")
                            .primary()
                            .disabled(!valid || waiting)
                            .on_click(cx.listener(|this, _, _, cx| this.request_join(cx))),
                    ),
            )
            .children(self.error.clone().map(|e| {
                Label::new(e).text_sm().text_color(crate::palette::danger())
            }))
    }

    /// Step 2: approve on the other device. Live while waiting, with the
    /// countdown; after a timeout it offers to keep waiting.
    fn render_approve(&self, my_name: &str, short: &str, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let active = !matches!(self.phase, Phase::Idle);
        let mut c = card(cx)
            .when(!active, |c| c.opacity(0.55))
            .child(step_title(2, "Approve it on that device", cx))
            .child(
                Label::new(format!(
                    "Its Devices page shows a request from \"{my_name}\". Check that \
                     the device ID there matches this one, then press Approve."
                ))
                .text_sm()
                .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(Label::new("This device's ID").text_sm().text_color(theme.muted_foreground))
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .rounded(px(6.))
                            .bg(theme.background)
                            .border_1()
                            .border_color(theme.border)
                            .child(Label::new(short.to_string()).text_sm()),
                    ),
            );

        match self.phase {
            Phase::Waiting { since } => {
                let left = JOIN_TIMEOUT_SECS.saturating_sub(since.elapsed().as_secs());
                let elapsed_pct = since.elapsed().as_secs_f32() * 100.0 / JOIN_TIMEOUT_SECS as f32;
                c = c
                    .child(Progress::new("join-wait").value(elapsed_pct.min(100.0)))
                    .child(
                        Label::new(format!(
                            "Waiting for approval, {}:{:02} left. This computer connects \
                             by itself once approved.",
                            left / 60,
                            left % 60
                        ))
                        .text_sm()
                        .text_color(theme.muted_foreground),
                    );
            }
            Phase::TimedOut => {
                c = c.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .child(
                            Label::new("No approval yet. The request may not have been seen.")
                                .text_sm()
                                .flex_1(),
                        )
                        .child(
                            Button::new("keep-waiting")
                                .label("Keep waiting")
                                .primary()
                                .compact()
                                .on_click(cx.listener(|this, _, _, cx| this.request_join(cx))),
                        ),
                );
            }
            Phase::Idle => {}
        }
        c
    }

    /// Collapsed help for inviting from a CLI member: this device's QR and
    /// a copy action, plus the command that uses it.
    fn render_cli(&self, id: &str, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let mut section = div().flex().flex_col().gap_3().w_full().child(
            div()
                .id("toggle-cli")
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .cursor_pointer()
                .text_color(theme.muted_foreground)
                .hover(|s| s.text_color(theme.foreground))
                .child(
                    Icon::new(if self.show_cli { IconName::ChevronDown } else { IconName::ChevronRight })
                        .with_size(px(14.)),
                )
                .child(Label::new("Inviting from the Noscat CLI instead?").text_sm())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.show_cli = !this.show_cli;
                    cx.notify();
                })),
        );
        if !self.show_cli {
            return section;
        }
        let qr = qr_image(id);
        section = section.child(
            card(cx)
                .flex_row()
                .items_center()
                .gap_5()
                .children(qr.map(|qr| {
                    div().flex_shrink_0().rounded(px(6.)).overflow_hidden().child(img(qr).size(px(148.)))
                }))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .min_w_0()
                        .child(
                            Label::new(
                                "Request to join above first, so this computer is \
                                 waiting. Then, on the CLI device, run:",
                            )
                            .text_sm()
                            .text_color(theme.muted_foreground),
                        )
                        .child(
                            div()
                                .px_2()
                                .py_1()
                                .rounded(px(6.))
                                .bg(theme.background)
                                .border_1()
                                .border_color(theme.border)
                                .child(Label::new("noscat mesh invite <device ID>").text_sm()),
                        )
                        .child(
                            div().child(
                                Button::new("copy-id")
                                    .icon(if self.copied { IconName::Check } else { IconName::Copy })
                                    .label(if self.copied { "Copied" } else { "Copy device ID" })
                                    .compact()
                                    .on_click(cx.listener({
                                        let id = id.to_string();
                                        move |this, _, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(id.clone()));
                                            this.copied = true;
                                            cx.notify();
                                        }
                                    })),
                            ),
                        ),
                ),
        );
        section
    }

    /// A device already in a mesh, offline: one button back in. While it
    /// connects (on its own at launch, or after Connect), it says so.
    fn render_reconnect(&self, starting: bool, mesh: Option<String>, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        if starting {
            let target = mesh.map(|m| format!("Connecting to {m}...")).unwrap_or_else(|| "Connecting...".into());
            return div()
                .flex()
                .flex_col()
                .items_center()
                .gap_3()
                .pt_16()
                .child(Spinner::new().with_size(px(28.)).color(theme.muted_foreground))
                .child(crate::ui::page_title(target))
                .child(
                    Label::new("Reaching the mesh's relays and your devices.")
                        .text_sm()
                        .text_color(theme.muted_foreground),
                );
        }
        div()
            .pt_16()
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .child(Icon::new(IconName::WifiOff).with_size(px(32.)).text_color(theme.muted_foreground))
            .child(crate::ui::page_title("Offline"))
            .child(
                Label::new("This computer is still in its mesh. Connect to reach your devices again.")
                    .text_sm()
                    .text_color(theme.muted_foreground),
            )
            .child(
                Button::new("connect")
                    .label(if starting { "Connecting..." } else { "Connect" })
                    .primary()
                    .disabled(starting)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |s, cx| s.start_node(cx));
                    })),
            )
    }
}

impl Render for JoinView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let has_mesh = !session.roster.is_empty();
        let starting = session.starting;
        let id = session.my_npub.clone().unwrap_or_default();
        let short = crate::state::short_id(&id);
        let my_name = session.settings.device_name.clone();
        let mesh_name = session.settings.mesh_name.clone();
        let theme = cx.theme().clone();

        let column = div()
            .flex()
            .flex_col()
            .items_center()
            .gap_4()
            .w_full()
            .max_w(px(560.))
            .mx_auto()
            .px_4()
            .py_10();

        let column = if has_mesh {
            column.child(self.render_reconnect(starting, mesh_name, cx))
        } else {
            let paste = self.render_paste(cx);
            let approve = self.render_approve(&my_name, &short, cx);
            let cli = self.render_cli(&id, cx);
            column
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_1()
                        .pb_2()
                        .child(crate::ui::page_title("Join a mesh"))
                        .child(
                            Label::new("A device already in the mesh lets this one in.")
                                .text_sm()
                                .text_color(theme.muted_foreground),
                        ),
                )
                .child(paste)
                .child(approve)
                .child(cli)
        };

        div()
            .id("join-flow")
            .size_full()
            .overflow_y_scroll()
            .child(column)
    }
}
