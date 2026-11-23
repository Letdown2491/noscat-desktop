//! Services tab (desktop extra): expose a local TCP service to peers, the
//! `noscat expose` shape, plus the live port-forward list.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::Disableable as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::gpui::{
    div, px, rgb, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_kit_assets::IconName;

use crate::state::{RunMode, Session};

pub struct ServicesView {
    session: Entity<Session>,
    expose_target: Entity<InputState>,
    /// Inline validation error for the expose target field.
    error: Option<String>,
}

impl ServicesView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let expose_target = cx.new(|cx| {
            InputState::new(window, cx).placeholder("local service, e.g. 127.0.0.1:8080")
        });
        Self {
            session,
            expose_target,
            error: None,
        }
    }
}

impl Render for ServicesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let theme = cx.theme().clone();
        let exposing = session.mode == RunMode::Expose;

        // Expose section: input plus start button (disabled while running -
        // expose holds the UDP listener role exclusively).
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
                    "Peers can reach this machine's service through their \
                     client while expose runs.",
                )
                .text_sm()
                .text_color(theme.muted_foreground),
            )
            .child(if let Some(target) = &session.expose_target {
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .py_1()
                    .child(
                        div().rounded_full().size(px(8.)).bg(rgb(0x22c55e)).flex_shrink_0(),
                    )
                    .child(Label::new(format!("Serving {target} to the mesh")))
                    .into_any_element()
            } else {
                div().into_any_element()
            })
            .child(
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
                                        this.error = None;
                                        this.session
                                            .update(cx, |s, cx| s.start_expose(addr, cx));
                                    }
                                    Err(_) => {
                                        this.error =
                                            Some("Use ip:port, e.g. 127.0.0.1:8080".into());
                                    }
                                }
                                cx.notify();
                            })),
                    ),
            )
            .child(match &self.error {
                Some(err) => Label::new(err.clone())
                    .text_sm()
                    .text_color(gpui_kit::gpui::Hsla::from(rgb(0xef4444)))
                    .into_any_element(),
                None => div().into_any_element(),
            });

        // Forward list (started from device rows). Upstream 0.4.0 note:
        // cancel_forward arrives there, so rows gain a Stop button.
        let mut forwards = div().flex().flex_col().gap_2().px_4().py_3();
        forwards = forwards.child(Label::new("Port forwards"));
        if session.forwards.is_empty() {
            forwards = forwards.child(
                Label::new("No forwards. Start one from a device row on the Devices tab.")
                    .text_sm()
                    .text_color(theme.muted_foreground),
            );
        }
        for f in session.forwards.clone() {
            let peer = session
                .roster
                .iter()
                .find(|d| d.npub == f.peer)
                .map(|d| d.display_name())
                .unwrap_or_else(|| "device".to_string());
            forwards = forwards.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(Label::new(format!(
                        "localhost:{} -> {peer}",
                        f.local_port
                    )))
                    .child(
                        div()
                            .ml_auto()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .rounded_full()
                                    .size(px(8.))
                                    .bg(rgb(0x22c55e))
                                    .flex_shrink_0(),
                            )
                            .child(
                                Label::new("active")
                                    .text_sm()
                                    .text_color(theme.muted_foreground),
                            )
                            // Stop the forward (client 0.4.0's
                            // cancel_forward): the row leaves when the
                            // confirming ForwardStopped event lands.
                            .child(
                                Button::new(SharedString::from(format!(
                                    "stop-forward-{}",
                                    f.id
                                )))
                                .icon(IconName::Square)
                                .ghost()
                                .compact()
                                .tooltip("Stop")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.session.update(cx, |s, cx| {
                                        s.stop_forward(f.id, cx);
                                    });
                                })),
                            ),
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
