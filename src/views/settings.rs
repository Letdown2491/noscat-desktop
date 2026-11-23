//! Settings tab: connection status, device name editing, appearance, storage
//! locations, the device ID copy action, and disconnect behind a two-step
//! confirm.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::gpui::{
    div, px, AppContext as _, ClipboardItem, Context, Div, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window,
};

use crate::app::Tab;
use crate::paths;
use crate::state::{RunMode, Session};

pub struct SettingsView {
    session: Entity<Session>,
    name_input: Entity<InputState>,
    /// The device ID was copied to the clipboard.
    copied: bool,
}

/// One titled group of settings rows, in the readable column.
fn section(title: &str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .px_4()
        .child(Label::new(title))
}

impl SettingsView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let initial = session.read(cx).settings.device_name.clone();
        let name_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("device name");
            state.set_value(initial, window, cx);
            state
        });
        Self {
            session,
            name_input,
            copied: false,
        }
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let theme = cx.theme().clone();
        let connected = session.mode != RunMode::Stopped;
        let npub = session.my_npub.clone().unwrap_or_else(|| "-".to_string());
        let confirm = session.confirm_disconnect;

        // Status: what the app is doing right now, in the user's language.
        // The UDP port stays visible: it is the one number worth reading
        // when a firewall question comes up.
        let status_line = if connected {
            match session.mode {
                RunMode::Expose => "connected, serving a local service".to_string(),
                _ => "connected".to_string(),
            }
        } else {
            "not connected".to_string()
        };
        let status = section("Status")
            .child(Label::new(status_line))
            .children(session.listen_port.map(|port| {
                Label::new(format!("listening on UDP port {port}"))
                    .text_sm()
                    .text_color(theme.muted_foreground)
            }))
            .child(
                Label::new(format!(
                    "{} devices in the mesh (the roster syncs every 2 minutes)",
                    session.roster.len()
                ))
                .text_sm()
                .text_color(theme.muted_foreground),
            );

        // Device name editor: persists to settings.json; the client picks it
        // up on the next advert/start.
        let name = section("Device name").child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(div().w_1_2().min_w_64().child(Input::new(&self.name_input)))
                .child(
                    Button::new("save-name")
                        .label("Save")
                        .compact()
                        .on_click(cx.listener(|this, _, _, cx| {
                            let name = this.name_input.read(cx).value().trim().to_string();
                            this.session.update(cx, |s, cx| s.set_device_name(name, cx));
                        })),
                ),
        );

        // Appearance: system / light / dark, segmented. System follows the
        // desktop's theme (the default).
        let theme_pref = session.settings.theme;
        let appearance = section("Appearance").child(
            div().flex().flex_row().gap_1().children({
                let mut buttons = Vec::new();
                for (pref, label, id) in [
                    (crate::paths::ThemePref::System, "System", "theme-system"),
                    (crate::paths::ThemePref::Light, "Light", "theme-light"),
                    (crate::paths::ThemePref::Dark, "Dark", "theme-dark"),
                ] {
                    let button = Button::new(id).label(label).compact();
                    let button = if theme_pref == pref {
                        button.primary()
                    } else {
                        button
                    };
                    buttons.push(button.on_click(cx.listener(move |this, _, window, cx| {
                        this.session.update(cx, |s, cx| {
                            s.set_theme(pref, cx);
                        });
                        crate::state::apply_theme(pref, Some(window), cx);
                        cx.notify();
                    })));
                }
                buttons
            }),
        );

        // Storage: where received files and identity data live.
        let storage = section("Storage")
            .child(
                Label::new(format!("received files: {}", paths::inbox_dir().display()))
                    .text_sm()
                    .truncate()
                    .text_color(theme.muted_foreground),
            )
            .child(
                Label::new(format!(
                    "device data: {}",
                    paths::data_dir().display()
                ))
                .text_sm()
                .truncate()
                .text_color(theme.muted_foreground),
            );

        // Device ID (an npub underneath): shown only as a copy action, never
        // printed - identity strings are not user information, matching the
        // Android app. The row explains when sharing is needed.
        let device_id = section("Device ID").child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    Label::new(
                        "Share this with a mesh owner when they invite this \
                         device.",
                    )
                    .text_sm()
                    .text_color(theme.muted_foreground),
                )
                .child(
                    Button::new("copy-npub")
                        .label(if self.copied { "Copied" } else { "Copy" })
                        .compact()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(npub.clone()));
                            this.copied = true;
                            cx.notify();
                        })),
                ),
        );

        div()
            .flex()
            .flex_col()
            .size_full()
            .id("settings-list")
            .overflow_y_scroll()
            .child(
                // A readable column: settings stretched across a wide
                // window read like a spreadsheet.
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .max_w(px(560.))
                    .py_3()
                    // Offline, Settings is reachable from the sidebar; give
                    // the way back to the connect screen.
                    .child(if session.mode == RunMode::Stopped {
                        div().px_4().child(
                            Button::new("back-connect")
                                .label("< Back to connect")
                                .compact()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.session
                                        .update(cx, |s, cx| s.set_tab(Tab::Devices, cx));
                                })),
                        )
                    } else {
                        div()
                    })
                    .child(status)
                    .child(name)
                    .child(appearance)
                    .child(storage)
                    .child(device_id)
                    .child(
                        div().px_4().pt_1().child(
                            // Two-step confirm: the first click arms the
                            // button, the second disconnects (the Android
                            // app uses a dialog).
                            Button::new("disconnect")
                                .label(if confirm {
                                    "Confirm disconnect?"
                                } else {
                                    "Disconnect"
                                })
                                .danger()
                                .disabled(!confirm && session.mode == RunMode::Stopped)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.session.update(cx, |s, cx| {
                                        if s.confirm_disconnect {
                                            s.disconnect(cx);
                                        } else {
                                            s.confirm_disconnect = true;
                                            cx.notify();
                                        }
                                    });
                                })),
                        ),
                    ),
            )
            .into_any_element()
    }
}
