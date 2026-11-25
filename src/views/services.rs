//! Services tab (desktop extra): expose a local TCP service to members
//! (node mode with a bridge, so files and chat keep working), and the port
//! forwards: each one makes a port on this computer reach the service
//! another device shares. Forwards start here or from a device row.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::gpui::{
    div, px, AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_kit_assets::IconName;

use crate::palette;
use crate::state::{RunMode, Session};

pub struct ServicesView {
    session: Entity<Session>,
    expose_target: Entity<InputState>,
    /// Inline validation error for the expose target field.
    error: Option<String>,
    /// The New forward editor: its port field, the chosen device, and its
    /// validation error. `None` while closed.
    new_forward: Option<(Entity<InputState>, Option<String>, Option<String>)>,
    /// The forward whose address was just copied.
    copied: Option<u64>,
}

impl ServicesView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let expose_target = cx.new(|cx| {
            InputState::new(window, cx).placeholder("local service, e.g. 127.0.0.1:8080")
        });
        cx.subscribe_in(&expose_target, window, |this, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.expose(cx);
            }
        })
        .detach();
        Self {
            session,
            expose_target,
            error: None,
            new_forward: None,
            copied: None,
        }
    }

    fn expose(&mut self, cx: &mut Context<Self>) {
        let target = self.expose_target.read(cx).value().trim().to_string();
        match target.parse::<std::net::SocketAddr>() {
            Ok(addr) => {
                self.error = None;
                self.session.update(cx, |s, cx| s.start_expose(addr, cx));
            }
            Err(_) => self.error = Some("Use ip:port, e.g. 127.0.0.1:8080".into()),
        }
        cx.notify();
    }

    fn open_new_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("e.g. 8080"));
        cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.start_new_forward(cx);
            }
        })
        .detach();
        self.new_forward = Some((input, None, None));
        cx.notify();
    }

    fn start_new_forward(&mut self, cx: &mut Context<Self>) {
        let Some((input, peer, _)) = self.new_forward.clone() else { return };
        let port = input.read(cx).value().trim().parse::<u16>().ok().filter(|p| *p > 0);
        let error = match (&peer, port) {
            (None, _) => Some("Choose a device".to_string()),
            (_, None) => Some("Enter a port number, e.g. 8080".to_string()),
            (Some(peer), Some(port)) => {
                let peer = peer.clone();
                self.new_forward = None;
                self.session.update(cx, |s, cx| s.add_forward(&peer, port, cx));
                None
            }
        };
        if let (Some(error), Some(editor)) = (error, self.new_forward.as_mut()) {
            editor.2 = Some(error);
        }
        cx.notify();
    }
}

impl Render for ServicesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let theme = cx.theme().clone();
        let exposing = session.mode == RunMode::Expose;
        let starting = session.starting;

        // Expose: while serving, what is served and Stop; otherwise the
        // target field and Expose.
        let body = if let (true, Some(target)) = (exposing, session.expose_target.clone()) {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(div().rounded_full().size(px(8.)).bg(palette::success()).flex_shrink_0())
                .child(Label::new(format!("Serving {target} to the mesh")).flex_1())
                .child(
                    Button::new("stop-expose")
                        .icon(IconName::Square)
                        .label(if starting { "Stopping..." } else { "Stop" })
                        .compact()
                        .disabled(starting)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.session.update(cx, |s, cx| s.stop_expose(cx));
                        })),
                )
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(div().w_1_2().min_w_64().child(Input::new(&self.expose_target)))
                .child(
                    Button::new("expose")
                        .label(if starting { "Starting..." } else { "Expose" })
                        .primary()
                        .disabled(starting)
                        .on_click(cx.listener(|this, _, _, cx| this.expose(cx))),
                )
                .into_any_element()
        };
        let expose = div()
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .py_3()
            .border_b_1()
            .border_color(theme.border)
            .child(Label::new("Expose a local service"))
            .child(
                Label::new(
                    "Members reach it by forwarding a port to this computer, from their \
                     Devices page or their own Services tab. Chat and files keep working.",
                )
                .text_sm()
                .text_color(theme.muted_foreground),
            )
            .child(body)
            .children(self.error.clone().map(|err| Label::new(err).text_sm().text_color(palette::danger())));

        // Forwards: the header row with New forward, the editor when open,
        // then one row per forward.
        let my_npub = session.my_npub.clone();
        let others: Vec<(String, String)> = session
            .roster
            .iter()
            .filter(|d| Some(&d.npub) != my_npub.as_ref())
            .map(|d| (d.npub.clone(), d.display_name()))
            .collect();
        let mut forwards = div().flex().flex_col().gap_2().px_4().py_3().child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .child(Label::new("Port forwards").flex_1())
                .child(
                    Button::new("new-forward")
                        .icon(IconName::Plus)
                        .label("New forward")
                        .compact()
                        .disabled(self.new_forward.is_some() || others.is_empty())
                        .on_click(cx.listener(|this, _, window, cx| this.open_new_forward(window, cx))),
                ),
        );

        if let Some((input, chosen, error)) = self.new_forward.clone() {
            let mut devices = div().flex().flex_row().flex_wrap().gap_1();
            for (npub, name) in others {
                let selected = chosen.as_deref() == Some(npub.as_str());
                let button = Button::new(SharedString::from(format!("pick-{npub}")))
                    .label(name)
                    .compact();
                let button = if selected { button.primary() } else { button };
                devices = devices.child(button.on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(editor) = this.new_forward.as_mut() {
                        editor.1 = Some(npub.clone());
                        editor.2 = None;
                    }
                    cx.notify();
                })));
            }
            forwards = forwards.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.secondary)
                    .child(
                        Label::new("Which device's shared service?")
                            .text_sm()
                            .text_color(theme.muted_foreground),
                    )
                    .child(devices)
                    .child(
                        Label::new("Port on this computer that reaches it")
                            .text_sm()
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(div().w_24().child(Input::new(&input)))
                            .child(
                                Button::new("start-new-forward")
                                    .label("Start forward")
                                    .primary()
                                    .compact()
                                    .on_click(cx.listener(|this, _, _, cx| this.start_new_forward(cx))),
                            )
                            .child(
                                Button::new("cancel-new-forward")
                                    .label("Cancel")
                                    .compact()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.new_forward = None;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .children(error.map(|e| Label::new(e).text_sm().text_color(palette::danger()))),
            );
        }

        if session.forwards.is_empty() && self.new_forward.is_none() {
            forwards = forwards.child(
                Label::new(
                    "No forwards. A forward makes a port on this computer reach the service \
                     another device shares.",
                )
                .text_sm()
                .text_color(theme.muted_foreground),
            );
        }
        for f in session.forwards.clone() {
            let peer = session.peer_name(&f.peer);
            let address = format!("localhost:{}", f.local_port);
            let copied = self.copied == Some(f.id);
            forwards = forwards.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .py_1()
                    .child(div().rounded_full().size(px(8.)).bg(palette::success()).flex_shrink_0())
                    .child(Label::new(address.clone()))
                    .child(
                        Button::new(SharedString::from(format!("copy-forward-{}", f.id)))
                            .icon(if copied { IconName::Check } else { IconName::Copy })
                            .ghost()
                            .xsmall()
                            .tooltip("Copy address")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(address.clone()));
                                this.copied = Some(f.id);
                                cx.notify();
                            })),
                    )
                    .child(
                        Label::new(format!("reaches {peer}'s shared service"))
                            .text_sm()
                            .flex_1()
                            .truncate()
                            .text_color(theme.muted_foreground),
                    )
                    // Stop the forward: the row leaves when the confirming
                    // ForwardStopped event lands.
                    .child(
                        Button::new(SharedString::from(format!("stop-forward-{}", f.id)))
                            .icon(IconName::Square)
                            .ghost()
                            .compact()
                            .tooltip("Stop")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.session.update(cx, |s, cx| s.stop_forward(f.id, cx));
                            })),
                    ),
            );
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .id("shares-list")
            .overflow_y_scroll()
            .child(expose)
            .child(forwards)
            .into_any_element()
    }
}
