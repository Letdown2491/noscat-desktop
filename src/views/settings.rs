//! Settings tab: connection status, device name editing, appearance, storage
//! locations, the device ID copy action, and disconnect behind a two-step
//! confirm.

use gpui_kit::gpui::prelude::FluentBuilder as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::gpui::{
    div, img, px, rgb, AppContext as _, ClipboardItem, Context, Div, Entity,
    InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Window,
};

use crate::app::Tab;
use crate::paths;
use crate::state::{RunMode, Session};

pub struct SettingsView {
    session: Entity<Session>,
    name_input: Entity<InputState>,
    /// The device ID was copied to the clipboard.
    copied: bool,
    /// The pairing QR is showing (the banner the Listening event carries;
    /// a member can scan or copy it to invite this device).
    show_qr: bool,
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
            show_qr: false,
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
        let confirm_forget = session.confirm_forget;
        let forgetting = session.forgetting;

        // Status: one summary line, no heading - the sidebar footer carries
        // the connection state already, so this adds the fact the footer
        // cannot hold: how much of the mesh is reachable. Offline, the same
        // slot says so, keeping the page from reshuffling.
        let my_npub = session.my_npub.as_ref();
        let reachable_count =
            session.roster.iter().filter(|d| d.connected || Some(&d.npub) == my_npub).count();
        let known = session.roster.len();
        let status_line = if connected {
            format!("Connected - {reachable_count} of {known} devices connected")
        } else {
            "Not connected".to_string()
        };
        // Two-step confirms: the first click arms a button, the second
        // acts (the Android app uses a dialog).
        let disconnect = Button::new("disconnect")
            .label(if confirm {
                "Confirm disconnect?"
            } else {
                "Disconnect"
            })
            .danger()
            .disabled(forgetting || (!confirm && session.mode == RunMode::Stopped))
            .on_click(cx.listener(|this, _, _, cx| {
                this.session.update(cx, |s, cx| {
                    if s.confirm_disconnect {
                        s.disconnect(cx);
                    } else {
                        s.confirm_disconnect = true;
                        s.confirm_forget = false;
                        cx.notify();
                    }
                });
            }));
        // Works while stopped too: leaving runs the device just for that.
        let forget = Button::new("forget")
            .label(if forgetting {
                "Leaving the mesh..."
            } else if confirm_forget {
                "Confirm forget?"
            } else {
                "Forget mesh"
            })
            .danger()
            .disabled(forgetting)
            .on_click(cx.listener(|this, _, _, cx| {
                this.session.update(cx, |s, cx| {
                    if s.confirm_forget {
                        s.forget(cx);
                    } else {
                        s.confirm_forget = true;
                        s.confirm_disconnect = false;
                        cx.notify();
                    }
                });
            }));

        let status = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .px_4()
            .py_3()
            .child(
                div().flex_shrink_0().rounded_full().size(px(8.)).bg(if connected {
                    rgb(0x22c55e)
                } else {
                    rgb(0xef4444)
                }),
            )
            .child(Label::new(status_line));

        // Diagnostics: the numbers worth having when something is wrong,
        // kept out of the status line's way. The bound UDP port answers
        // firewall questions; diagnostics grow here.
        // How the device files are kept (identity, mesh keys).
        let storage = match crate::keystore::status() {
            crate::keystore::Storage::Sealed => "device keys encrypted, key in the system keyring".to_string(),
            crate::keystore::Storage::Plain(why) => format!("device keys stored unencrypted: {why}"),
            crate::keystore::Storage::Locked(why) => {
                format!("device keys are encrypted but the key is unavailable: {why}")
            }
        };
        let diagnostics = Some(
            section("Diagnostics")
                .when(connected, |s| {
                    s.child(
                        Label::new(match session.listen_port {
                            Some(port) => format!("bound to UDP port {port}"),
                            None => "no UDP port bound yet".to_string(),
                        })
                        .text_sm()
                        .text_color(theme.muted_foreground),
                    )
                })
                .child(Label::new(storage).text_sm().text_color(theme.muted_foreground)),
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

        // Reachable: this device as the mesh's meeting point for devices
        // on other networks. Needs a router port forward the app cannot
        // check, so the text says what to do and how to tell it worked.
        let reachable_on = session.settings.reachable;
        let network = section("Reachable from the internet")
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        Switch::new("reachable")
                            .checked(reachable_on)
                            .disabled(session.starting)
                            .on_click({
                                let session = self.session.clone();
                                move |on, _, cx| {
                                    let on = *on;
                                    session.update(cx, |s, cx| s.set_reachable(on, cx));
                                }
                            }),
                    )
                    .child(Label::new(if reachable_on { "On" } else { "Off" }).text_sm()),
            )
            .child(
                Label::new(
                    "Makes this device the meeting point for devices on other networks \
                     (a phone on mobile data, a friend elsewhere): one per mesh is enough. \
                     Forward UDP port 41641 on your router to this computer, and keep it \
                     running. Relayed traffic uses its upload bandwidth.",
                )
                .text_sm()
                .text_color(theme.muted_foreground),
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

        // Device ID (an npub underneath): shown as a scannable QR (the
        // pairing banner the Listening event carries) plus a copy action, never
        // printed - identity strings are not user information, matching
        // the Android app. The row explains when sharing is needed.
        let mut device_id = section("Device ID").child(
            Label::new("Share this with a mesh member when they invite this device.")
                .text_sm()
                .text_color(theme.muted_foreground),
        );
        if self.show_qr {
            // In a private mesh the QR also names its relays, so a new
            // device joins through them.
            if let Some(npub) = session.pairing_uri.as_ref().or(session.my_npub.as_ref()) {
                if let Some(qr) = crate::views::join::qr_image(npub) {
                    device_id = device_id.child(
                        div().p_3().border_1().border_color(theme.border).rounded(px(10.)).bg(theme.secondary).child(
                            img(qr).size(px(176.)),
                        ),
                    );
                }
            }
        }
        device_id = device_id.child(
            div()
                .flex()
                .flex_row()
                .gap_2()
                .child(
                    Button::new("toggle-qr")
                        .label(if self.show_qr { "Hide QR" } else { "Show QR" })
                        .compact()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_qr = !this.show_qr;
                            cx.notify();
                        })),
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
                // window read like a spreadsheet. No top padding of its
                // own: content starts at the same height as the other
                // pages' first row.
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .max_w(px(560.))
                    // Offline, Settings is reachable from the sidebar; give
                    // the way back to the connect screen. Conditional child,
                    // not an empty div: a filler child would still earn a
                    // gap_4 of blank space above the first section.
                    .children((session.mode == RunMode::Stopped).then(|| {
                        div().px_4().child(
                            Button::new("back-connect")
                                .label("< Back to connect")
                                .compact()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.session
                                        .update(cx, |s, cx| s.set_tab(Tab::Devices, cx));
                                })),
                        )
                    }))
                    .child(status)
                    .child(name)
                    .child(network)
                    .child(appearance)
                    .child(storage)
                    .child(device_id)
                    .children(diagnostics)
                    .child(
                        div()
                            .px_4()
                            .pt_1()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                Label::new(
                                    "Disconnect goes offline and keeps this device in the \
                                     mesh. Forget asks the other members to remove it and \
                                     deletes its identity and mesh data here.",
                                )
                                .text_sm()
                                .text_color(theme.muted_foreground),
                            )
                            .child(div().flex().gap_2().child(disconnect).child(forget)),
                    ),
            )
            .into_any_element()
    }
}
