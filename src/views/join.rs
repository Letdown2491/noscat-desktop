//! Join flow: the desktop counterpart of the Android app's QR join. The
//! desktop has no camera, so the flow is paste and display: paste the
//! owner's npub (the one whose QR was scanned on the phone) to address the
//! invite request, and show this device's own npub (and QR) for the owner
//! to invite it back.
//!
//! The flow is one button: publish our MLS key package addressed to the
//! owner, then wait up to two minutes for the gift-wrapped invitation. On
//! success the session starts in node mode. Re-running with an existing
//! mesh membership skips straight to connecting.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::Disableable as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::gpui::{
    div, px, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Window,
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
    Joining,
}

pub struct JoinView {
    session: Entity<Session>,
    owner_input: Entity<InputState>,
    phase: Phase,
    /// Progress or error text: (is_error, message).
    status: Option<(bool, String)>,
}

impl JoinView {
    pub fn new(
        session: Entity<Session>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let owner_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("npub of the mesh owner (optional)")
        });
        Self {
            session,
            owner_input,
            phase: Phase::Idle,
            status: None,
        }
    }

    fn set_status(&mut self, is_error: bool, text: impl Into<String>, cx: &mut Context<Self>) {
        self.status = Some((is_error, text.into()));
        cx.notify();
    }

    /// Run the whole flow in the background: prepare (publish the key
    /// package), then wait for the invitation, then start the node.
    fn start(&mut self, cx: &mut Context<Self>) {
        let owner_raw = self.owner_input.read(cx).value().trim().to_string();
        let owner = (!owner_raw.is_empty()).then_some(owner_raw);
        let dir = paths::mesh_dir();

        // A device that already joined this mesh (the roster persisted)
        // skips the invite dance and connects directly.
        if !self.session.read(cx).roster.is_empty() {
            self.session.update(cx, |s, cx| s.start_node(cx));
            return;
        }

        self.phase = Phase::Preparing;
        self.set_status(false, "publishing our key package on the relays...", cx);

        let session = self.session.clone();
        cx.spawn(async move |this, cx| {
            let prepared = cx.background_executor().spawn({
                let dir = dir.clone();
                let owner = owner.clone();
                async move { mesh_helpers::prepare(&dir, owner.as_deref()) }
            });
            let npub = match prepared.await {
                Ok(npub) => npub,
                Err(e) => {
                    let _ = this.update(cx, |t, cx| {
                        t.phase = Phase::Idle;
                        t.set_status(true, format!("invite request failed: {e}"), cx)
                    });
                    return;
                }
            };
            let _ = this.update(cx, |t, cx| {
                t.phase = Phase::WaitingForInvite;
                t.set_status(
                    false,
                    format!("key package published as {npub}. Waiting for the owner's invite (up to 2 minutes)..."),
                    cx,
                )
            });

            let joined =
                cx.background_executor()
                    .spawn(async move { mesh_helpers::join(&dir, 120, owner.as_deref()) });
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
                        t.set_status(true, format!("join failed: {e}"), cx)
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
            .size_full()
            .items_center()
            .justify_center()
            .gap_3();

        column = column.child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .child(Label::new("Connect to a mesh").text_size(px(18.)))
                .child(
                    Label::new(if has_mesh {
                        "Your mesh membership was found on this device."
                    } else {
                        "Paste the owner's npub to request an invite, or show \
                         your npub below to the owner (the phone can scan it)."
                    })
                    .text_sm()
                    .text_color(theme.muted_foreground),
                ),
        );

        if has_mesh {
            column = column.child(
                Button::new("connect")
                    .label("Connect")
                    .primary()
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
            );
        } else {
            column = column
                .child(div().w_1_2().min_w_96().child(Input::new(&self.owner_input)))
                .child(
                    Button::new("join")
                        .label("Request invite and join")
                        .primary()
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
                );
        }

        if let Some((is_error, text)) = &self.status {
            column = column.child(
                Label::new(text.clone())
                    .text_sm()
                    .text_color(if *is_error {
                        gpui_kit::gpui::rgb(0xef4444).into()
                    } else {
                        theme.muted_foreground
                    }),
            );
        }

        column = column.child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .p_4()
                .border_1()
                .border_color(theme.border)
                .rounded(px(8.))
                .child(
                    Label::new("This device's npub:")
                        .text_sm()
                        .text_color(theme.muted_foreground),
                )
                .child(Label::new(npub)),
        );

        column.into_any_element()
    }
}
