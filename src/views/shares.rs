//! Shares tab (desktop extra): expose a local TCP service to peers, the
//! `noscat expose` shape, plus the live port-forward list.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::Disableable as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::gpui::{
    div, rgb, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window,
};

use crate::state::{RunMode, Session};

pub struct SharesView {
    session: Entity<Session>,
    expose_target: Entity<InputState>,
}

impl SharesView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let expose_target = cx.new(|cx| {
            InputState::new(window, cx).placeholder("local service, e.g. 127.0.0.1:8080")
        });
        Self {
            session,
            expose_target,
        }
    }
}

impl Render for SharesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let theme = cx.theme().clone();
        let exposing = session.mode == RunMode::Expose;

        // Expose section: input plus start button (disabled while running -
        // expose holds the UDP listener role exclusively).
        let mut expose = div()
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .child(Label::new("Expose a local service"))
            .child(
                Label::new(
                    "Peers can reach this machine's service through their \
                     client while expose runs.",
                )
                .text_sm()
                .text_color(theme.muted_foreground),
            );
        if let Some(target) = &session.expose_target {
            expose = expose.child(
                Label::new(format!("exposing {target}"))
                    .text_sm()
                    .text_color(rgb(0x22c55e)),
            );
        }
        expose = expose.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(div().w_1_2().min_w_64().child(Input::new(&self.expose_target)))
                .child(
                    Button::new("expose")
                        .label(if exposing { "Running" } else { "Expose" })
                        .primary()
                        .disabled(exposing)
                        .on_click(cx.listener(|this, _, _, cx| {
                            let target =
                                this.expose_target.read(cx).value().trim().to_string();
                            match target.parse::<std::net::SocketAddr>() {
                                Ok(addr) => {
                                    this.session.update(cx, |s, cx| s.start_expose(addr, cx))
                                }
                                Err(_) => this.session.update(cx, |s, _| {
                                    s.last_error =
                                        Some("target must be ip:port, e.g. 127.0.0.1:8080".into());
                                }),
                            }
                            cx.notify();
                        })),
                ),
        );

        // Forward list (started from device rows).
        let mut forwards = div().flex().flex_col().gap_2().px_4().py_2();
        forwards = forwards.child(Label::new("Port forwards"));
        if session.forwards.is_empty() {
            forwards = forwards.child(
                Label::new("No forwards. Start one from a device row on the Devices tab.")
                    .text_sm()
                    .text_color(theme.muted_foreground),
            );
        }
        for f in session.forwards.clone() {
            forwards = forwards.child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .child(Label::new(format!(
                        "localhost:{} -> {}",
                        f.local_port, f.peer
                    )))
                    .child(
                        Label::new("active")
                            .text_sm()
                            .text_color(rgb(0x22c55e)),
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
