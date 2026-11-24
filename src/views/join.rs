//! Join flow: the desktop counterpart of the Android app's QR join. The
//! desktop has no camera, so the flow is paste and display: paste a mesh
//! member's npub, and show this device's own npub (QR and copy) for the
//! member to invite.
//!
//! The flow is one button: `mesh::join` runs the device, sends the member a
//! join request and waits up to two minutes for the invitation from that
//! member only. On success the session starts in node mode. Re-running
//! with an existing mesh membership skips straight to connecting.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::Disableable as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::gpui::{
    div, px, img, rgb, AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, RenderImage, StatefulInteractiveElement as _,
    Styled as _, Window,
};

use crate::mesh as mesh_helpers;
use crate::paths;
use crate::state::Session;

/// How far through the join flow we are; drives the button and status text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Preparing,
    WaitingForInvite,
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

pub struct JoinView {
    session: Entity<Session>,
    owner_input: Entity<InputState>,
    phase: Phase,
    /// Progress or error text: (is_error, message).
    status: Option<(bool, String)>,
    /// The npub was copied to the clipboard (hint under the QR card).
    copied: bool,
}

impl JoinView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let owner_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("paste a mesh member's device ID (starts with npub1)")
        });
        let view = Self {
            session,
            owner_input,
            phase: Phase::Idle,
            status: None,
            copied: false,
        };
        // Start right away: with a mesh this reconnects, and without one the
        // status line says to paste a member's device ID.
        if view.session.read(cx).roster.is_empty() {
            cx.spawn(async move |this, cx| {
                let _ = this.update(cx, |t, cx| t.start(cx));
            })
            .detach();
        }
        view
    }

    fn set_status(&mut self, is_error: bool, text: impl Into<String>, cx: &mut Context<Self>) {
        self.status = Some((is_error, text.into()));
        self.copied = false;
        cx.notify();
    }

    /// Run the whole flow in the background: prepare (publish the key
    /// package), then wait for the invitation, then start the node.
    fn start(&mut self, cx: &mut Context<Self>) {
        let owner_raw = self.owner_input.read(cx).value().trim().to_string();

        // Accept the QR/deep-link form too (`noscat:<npub>?...`): a pasted
        // pairing URI normalizes to the member's npub before anything uses
        // it. Plain npubs (and anything else) pass through untouched.
        let owner_raw = match noscat_client::parse_pairing_target(&owner_raw) {
            Ok(pk) => {
                use nostr::nips::nip19::ToBech32 as _;
                pk.to_bech32().unwrap_or(owner_raw)
            }
            Err(_) => owner_raw,
        };

        // The member's device ID anchors the join: invitations from anyone
        // else are refused, and the join request goes to that member. A
        // fresh device (empty roster) must name it.
        let roster_empty = self.session.read(cx).roster.is_empty();
        let dir = paths::mesh_dir();
        if roster_empty && owner_raw.is_empty() {
            self.set_status(
                true,
                "Paste the device ID of a device already in the mesh first."
                    .to_string(),
                cx,
            );
            return;
        }
        let owner = (!owner_raw.is_empty()).then_some(owner_raw);

        // A device that already joined this mesh (the roster persisted)
        // skips the invite dance and connects directly.
        if !self.session.read(cx).roster.is_empty() {
            self.session.update(cx, |s, cx| s.start_node(cx));
            return;
        }

        self.phase = Phase::Preparing;
        self.set_status(false, "Asking the member to invite this device...", cx);

        let session = self.session.clone();
        cx.spawn(async move |this, cx| {
            let prepared = cx.background_executor().spawn({
                let dir = dir.clone();
                let owner = owner.clone();
                async move { mesh_helpers::prepare(&dir, owner.as_deref()) }
            });
            if let Err(e) = prepared.await {
                let _ = this.update(cx, |t, cx| {
                    t.phase = Phase::Idle;
                    t.set_status(true, format!("invite request failed: {e}"), cx)
                });
                return;
            }
            let _ = this.update(cx, |t, cx| {
                t.phase = Phase::WaitingForInvite;
                t.set_status(
                    false,
                    "Join request sent. Waiting for the invite (up to 2 \
                     minutes); the member approves it on their side...",
                    cx,
                )
            });

            // Joining needs the expected member. This point is only
            // reachable with an empty roster, which the guard above keeps
            // from being member-less.
            let join_owner = owner.clone().expect("owner required to join");
            let joined = cx.background_executor().spawn(async move {
                mesh_helpers::join(&dir, 120, &join_owner)
            });
            match joined.await {
                Ok(_) => {
                    let _ = this.update(cx, |t, cx| {
                        t.phase = Phase::Idle;
                        t.status = None;
                        // Connected: the session starts and the root view
                        // flips to the tabs.
                        session.update(cx, |s, cx| s.start_node(cx));
                    });
                }
                Err(e) => {
                    let _ = this.update(cx, |t, cx| {
                        t.phase = Phase::Idle;
                        let msg = e.to_string();
                        if msg.contains("no invitation arrived in time") {
                            // Normal case: the member has not approved yet.
                            // Not an error - the button asks again.
                            t.set_status(
                                false,
                                "No invite yet. On the member, approve the join \
                                 request (or run the command above), then try \
                                 again.",
                                cx,
                            )
                        } else {
                            t.set_status(true, format!("join failed: {e}"), cx)
                        }
                    });
                }
            }
        })
        .detach();
    }
}

impl Render for JoinView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let theme = cx.theme().clone();
        let npub = session.my_npub.clone().unwrap_or_else(|| "-".to_string());
        let has_mesh = !session.roster.is_empty();
        let busy = self.phase != Phase::Idle;

        let mut column = div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .size_full()
            .id("join-flow")
            .overflow_y_scroll();

        column = column.child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .mt_8()
                .child(Label::new(if has_mesh { "Reconnect" } else { "Join a mesh" }).text_size(px(20.)))
                .child(
                    Label::new(if has_mesh {
                        "This device already belongs to a mesh."
                    } else {
                        "A device already in the mesh invites this one - then \
                         it connects by itself."
                    })
                    .text_sm()
                    .text_color(theme.muted_foreground),
                ),
        );

        // Step 1: this device's ID, which a member invites. In 0.5 only the
        // CLI can invite, and only while this device waits in a join.
        // The npub never shows: like the Android app, identity is
        // something the app carries, not a string the user reads.
        if let Some(qr) = qr_image(&npub) {
            let card = div()
                .flex()
                .flex_col()
                .items_center()
                .gap_2()
                .px_6()
                .py_4()
                .border_1()
                .border_color(theme.border)
                .rounded(px(10.))
                .bg(theme.secondary)
                .max_w(px(560.))
                .child(Label::new("1. A member invites this device").text_size(px(15.)))
                .child(
                    Label::new("Start the join below first: this device must be \
                         waiting when the invite is sent.")
                        .text_sm()
                        .text_color(theme.muted_foreground),
                )
                .child(img(qr).size(px(176.)))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_1()
                        .child(
                            Label::new("The member approves the join request, or \
                                 runs this command with your device ID:")
                                .text_sm()
                                .text_color(theme.muted_foreground),
                        )
                        .child(
                            Button::new("copy-command")
                                .label(if self.copied {
                                    "Copied"
                                } else {
                                    "Copy device ID"
                                })
                                .compact()
                                .on_click(cx.listener({
                                    let npub = npub.clone();
                                    move |this, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            npub.clone(),
                                        ));
                                        this.copied = true;
                                        cx.notify();
                                    }
                                })),
                        )
                        .child(
                            Label::new("noscat mesh invite <device ID>")
                                .text_sm()
                                .text_color(theme.muted_foreground),
                        ),
                )
                .child(if self.copied {
                    Label::new("copied to clipboard")
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .into_any_element()
                } else {
                    div().into_any_element()
                });
            column = column.child(card);
        }

        // Step 2: what happens without any further clicks. Reassurance that
        // the wait is normal - the previous text left users guessing.
        if self.phase == Phase::WaitingForInvite {
            column = column.child(
                Label::new("2. The app connects automatically when the invite \
                     arrives")
                    .text_sm()
                    .text_color(theme.muted_foreground),
            );
        }

        // Joining by pasting a member's npub: sends it a join request and
        // waits for the invite.
        if !has_mesh {
            column = column.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .max_w(px(560.))
                    .child(Label::new("or").text_sm().text_color(theme.muted_foreground))
                    .child(
                        Label::new("Join by pasting a mesh member's device ID")
                            .text_sm()
                            .text_color(theme.muted_foreground),
                    )
                    .child(div().w(px(480.)).child(Input::new(&self.owner_input)))
                    .child(
                        Button::new("join")
                            .label("Request invite and join")
                            .primary()
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
                    ),
            );
        } else {
            column = column.child(
                Button::new("connect")
                    .label("Connect")
                    .primary()
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
            );
        }

        if let Some((is_error, text)) = &self.status {
            let color = if *is_error {
                gpui_kit::gpui::Hsla::from(rgb(0xef4444))
            } else {
                theme.muted_foreground
            };
            column = column.child(
                div().max_w(px(560.)).child(
                    Label::new(text.clone()).text_sm().text_color(color),
                ),
            );
        }

        column.into_any_element()
    }
}
