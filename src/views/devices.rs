//! Devices tab: the mesh roster with presence dots and per-row actions
//! (chat, send file, forward a local port), matching the Android
//! device list.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::label::Label;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::gpui::{
    div, px, rgb, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_kit_assets::IconName;

use crate::app::Tab;
use crate::state::Session;

pub struct DevicesView {
    session: Entity<Session>,
    /// The device whose forward editor is open, if any. Forwarding needs a
    /// local port, so the row expands an inline editor, like the Android
    /// dialog.
    forward_peer: Option<String>,
    forward_port: Option<Entity<gpui_kit::component::input::InputState>>,
    /// Inline validation error for the open forward editor.
    forward_error: Option<String>,
}

impl DevicesView {
    pub fn new(session: Entity<Session>, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            session,
            forward_peer: None,
            forward_port: None,
            forward_error: None,
        }
    }
}

impl Render for DevicesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let theme = cx.theme().clone();

        let mut list = div()
            .id("devices-list")
            .flex()
            .flex_col()
            .size_full()
            .overflow_y_scroll();

        if session.roster.is_empty() {
            list = list.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Label::new(
                        "No devices yet. Devices appear after the next mesh sync.",
                    )),
            );
        }

        for device in session.roster.clone() {
            let npub = device.npub.clone();
            let name = device.display_name();
            let online = device.online();
            let is_owner = device.is_owner;
            let presence = device.presence_text();
            let forward_open = self.forward_peer.as_deref() == Some(npub.as_str());

            let mut row = div()
                .id(SharedString::from(format!("device-row-{npub}")))
                .flex()
                .flex_col()
                .px_4()
                .py_3()
                .border_b_1()
                .border_color(theme.border)
                .hover(|s| s.bg(theme.secondary));

            // Avatar: the device's initial on a tinted disc, the Android
            // roster's shape. Presence is a small dot pinned to the avatar
            // edge, so the headline reads name-first.
            let initial = name
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_else(|| "?".to_string());
            let headline = div()
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .relative()
                        .flex_shrink_0()
                        .child(
                            div()
                                .rounded_full()
                                .size(px(36.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(theme.accent)
                                .child(
                                    Label::new(initial)
                                        .text_color(theme.accent_foreground),
                                ),
                        )
                        .child(
                            div()
                                .absolute()
                                .bottom_0()
                                .right_0()
                                .rounded_full()
                                .size(px(10.))
                                .border_2()
                                .border_color(theme.background)
                                .bg(if online {
                                    rgb(0x22c55e)
                                } else {
                                    rgb(0x94a3b8)
                                }),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(Label::new(if is_owner {
                            format!("{name} (owner)")
                        } else {
                            name
                        }))
                        .child(
                            Label::new(presence)
                                .text_sm()
                                .text_color(theme.muted_foreground),
                        ),
                );

            // Row actions as icon buttons with tooltips (desktop convention
            // for repeated list actions), pushed to the row's right edge.
            let actions = div()
                .flex()
                .flex_row()
                .gap_1()
                .ml_auto()
                .child(
                    Button::new(SharedString::from(format!("chat-{npub}")))
                        .icon(IconName::MessageSquare)
                        .ghost()
                        .tooltip("Chat")
                        .on_click(cx.listener({
                            let npub = npub.clone();
                            move |this, _, _, cx| {
                                this.forward_peer = None;
                                this.forward_port = None;
                                this.session.update(cx, |s, cx| {
                                    s.active_chat = Some(npub.clone());
                                    s.set_tab(Tab::Chat, cx);
                                });
                            }
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("file-{npub}")))
                        .icon(IconName::FolderOpen)
                        .ghost()
                        .tooltip("Send a file")
                        .on_click(cx.listener({
                            let npub = npub.clone();
                            move |this, _, _, cx| this.pick_and_send_file(&npub, cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("forward-{npub}")))
                        .icon(IconName::ArrowLeftRight)
                        .ghost()
                        .tooltip("Forward a local port")
                        .on_click(cx.listener({
                            let npub = npub.clone();
                            move |this, _, window, cx| {
                                this.open_forward_editor(npub.clone(), window, cx);
                            }
                        })),
                );
            row = row.child(headline.child(actions));

            // Inline forward editor for this row: port field with Enter
            // submit, and an inline validation error (never the header
            // chip - the error belongs next to the field).
            if forward_open {
                let port = self.forward_port.clone().expect("editor has an input");
                let mut editor = div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .pt_2()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                div().w_24().child(
                                    gpui_kit::component::input::Input::new(&port),
                                ),
                            )
                            .child(
                                Button::new(SharedString::from(format!(
                                    "start-forward-{npub}"
                                )))
                                .label("Forward local port")
                                .compact()
                                .primary()
                                .on_click(cx.listener({
                                    let npub = npub.clone();
                                    move |this, _, _, cx| {
                                        let port = port.read(cx).value().trim().to_string();
                                        match port.parse::<u16>() {
                                            Ok(port) => {
                                                this.forward_error = None;
                                                this.close_forward_editor();
                                                this.session.update(cx, |s, cx| {
                                                    s.add_forward(&npub, port, cx)
                                                });
                                            }
                                            Err(_) => {
                                                this.forward_error = Some(
                                                    "Enter a port number, e.g. 8080".into(),
                                                );
                                                cx.notify();
                                            }
                                        }
                                    }
                                })),
                            )
                            .child(
                                Button::new(SharedString::from(format!(
                                    "cancel-forward-{npub}"
                                )))
                                .label("Cancel")
                                .compact()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_forward_editor();
                                    cx.notify();
                                })),
                            ),
                    );
                if let Some(err) = &self.forward_error {
                    editor = editor.child(
                        Label::new(err.clone())
                            .text_sm()
                            .text_color(gpui_kit::gpui::Hsla::from(rgb(0xef4444))),
                    );
                }
                row = row.child(editor);
            }

            list = list.child(row);
        }

        list.into_any_element()
    }
}

impl DevicesView {
    fn close_forward_editor(&mut self) {
        self.forward_peer = None;
        self.forward_port = None;
    }

    fn open_forward_editor(
        &mut self,
        npub: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.forward_peer = Some(npub);
        self.forward_error = None;
        self.forward_port = Some(cx.new(|cx| {
            gpui_kit::component::input::InputState::new(window, cx)
                .placeholder("local port, e.g. 8080")
        }));
        cx.notify();
    }

    /// Open the portal file picker in the background and start the transfer.
    fn pick_and_send_file(&mut self, npub: &str, cx: &mut Context<Self>) {
        let npub = npub.to_string();
        cx.spawn(async move |this, cx| {
            let picked = cx
                .background_executor()
                .spawn(async move { rfd::FileDialog::new().pick_file() })
                .await;
            if let Some(path) = picked {
                let _ = this.update(cx, |this, cx| {
                    this.session.update(cx, |s, cx| s.send_file(&npub, path, cx));
                });
            }
        })
        .detach();
    }
}
