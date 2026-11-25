//! Settings tab: connection status, device name, reachability, appearance,
//! the device ID (QR and copy), storage and diagnostics, and the danger
//! section: Disconnect (reversible, so immediate) and Forget mesh (behind a
//! confirm dialog).

use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, WindowExt as _};
use gpui_kit::gpui::prelude::FluentBuilder as _;
use gpui_kit::gpui::{
    div, img, px, AppContext as _, ClipboardItem, Context, Div, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_kit_assets::IconName;

use crate::app::Tab;
use crate::palette;
use crate::paths;
use crate::state::{RunMode, Session};

/// How long "Saved" shows after a rename.
const SAVED_FOR: Duration = Duration::from_secs(2);

pub struct SettingsView {
    session: Entity<Session>,
    name_input: Entity<InputState>,
    /// The device ID was copied to the clipboard.
    copied: bool,
    /// The pairing QR is showing (a member can scan or copy it to invite
    /// this device).
    show_qr: bool,
    /// The reachability setup steps are open.
    show_reach_help: bool,
    /// When the name was last saved, for the "Saved" confirmation.
    saved_at: Option<Instant>,
}

/// One titled group of settings rows, in the readable column.
fn section(title: &str) -> Div {
    div().flex().flex_col().gap_2().px_4().child(Label::new(title.to_string()))
}

impl SettingsView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let initial = session.read(cx).settings.device_name.clone();
        let name_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("device name");
            state.set_value(initial, window, cx);
            state
        });
        // Save enables as the name changes; Enter saves.
        cx.subscribe_in(&name_input, window, |this, _, ev: &InputEvent, _, cx| match ev {
            InputEvent::Change => cx.notify(),
            InputEvent::PressEnter { .. } => this.save_name(cx),
            _ => {}
        })
        .detach();
        Self {
            session,
            name_input,
            copied: false,
            show_qr: false,
            show_reach_help: false,
            saved_at: None,
        }
    }

    fn save_name(&mut self, cx: &mut Context<Self>) {
        let name = self.name_input.read(cx).value().trim().to_string();
        if name.is_empty() || name == self.session.read(cx).settings.device_name {
            return;
        }
        self.session.update(cx, |s, cx| s.set_device_name(name, cx));
        self.saved_at = Some(Instant::now());
        cx.notify();
        // Take the confirmation down again.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVED_FOR).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    fn confirm_forget(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let session = self.session.clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let session = session.clone();
            alert
                .title("Forget this mesh?")
                .description(
                    "The other members are asked to remove this computer, then its identity, \
                     chats and mesh data here are deleted. Received files stay. To come back, \
                     join again as a new device.",
                )
                .show_cancel(true)
                .ok_text("Forget mesh")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, _, cx| {
                    session.update(cx, |s, cx| s.forget(cx));
                    true
                })
        });
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The Devices page's Rename opens Settings with the name focused.
        if self.session.read(cx).focus_name {
            self.session.update(cx, |s, _| s.focus_name = false);
            self.name_input.update(cx, |i, cx| i.focus(window, cx));
        }

        let session = self.session.read(cx);
        let theme = cx.theme().clone();
        let connected = session.mode != RunMode::Stopped;
        let npub = session.my_npub.clone().unwrap_or_else(|| "-".to_string());
        let forgetting = session.forgetting;

        // Status: how much of the mesh is reachable.
        let my_npub = session.my_npub.as_ref();
        let reachable_count =
            session.roster.iter().filter(|d| d.connected || Some(&d.npub) == my_npub).count();
        let known = session.roster.len();
        let status_line = if connected {
            format!("Connected: {reachable_count} of {known} devices")
        } else {
            "Not connected".to_string()
        };
        let status = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .px_4()
            .py_3()
            .child(div().flex_shrink_0().rounded_full().size(px(8.)).bg(if connected {
                palette::success()
            } else {
                palette::danger()
            }))
            .child(Label::new(status_line));

        // Device name: Save enables once it differs; "Saved" confirms.
        let typed = self.name_input.read(cx).value().trim().to_string();
        let changed = !typed.is_empty() && typed != session.settings.device_name;
        let saved = self.saved_at.is_some_and(|t| t.elapsed() < SAVED_FOR);
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
                        .disabled(!changed)
                        .on_click(cx.listener(|this, _, _, cx| this.save_name(cx))),
                )
                .when(saved && !changed, |row| {
                    row.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .text_color(palette::success())
                            .child(Icon::new(IconName::Check).with_size(px(14.)))
                            .child(Label::new("Saved").text_sm()),
                    )
                }),
        );

        // Appearance: system / light / dark, segmented.
        let theme_pref = session.settings.theme;
        let appearance = section("Appearance").child(div().flex().flex_row().gap_1().children({
            let mut buttons = Vec::new();
            for (pref, label, id) in [
                (paths::ThemePref::System, "System", "theme-system"),
                (paths::ThemePref::Light, "Light", "theme-light"),
                (paths::ThemePref::Dark, "Dark", "theme-dark"),
            ] {
                let button = Button::new(id).label(label).compact();
                let button = if theme_pref == pref { button.primary() } else { button };
                buttons.push(button.on_click(cx.listener(move |this, _, window, cx| {
                    this.session.update(cx, |s, cx| s.set_theme(pref, cx));
                    crate::state::apply_theme(pref, Some(window), cx);
                    cx.notify();
                })));
            }
            buttons
        }));

        // Reachable: one sentence, the setup behind a disclosure.
        let reachable_on = session.settings.reachable;
        let mut network = section("Reachable from the internet")
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
                    "Lets devices on other networks connect through this computer; one per \
                     mesh is enough.",
                )
                .text_sm()
                .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .id("reach-help")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .cursor_pointer()
                    .text_color(theme.muted_foreground)
                    .hover(|s| s.text_color(theme.foreground))
                    .child(
                        Icon::new(if self.show_reach_help { IconName::ChevronDown } else { IconName::ChevronRight })
                            .with_size(px(14.)),
                    )
                    .child(Label::new("How to set this up").text_sm())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_reach_help = !this.show_reach_help;
                        cx.notify();
                    })),
            );
        if self.show_reach_help {
            network = network.child(
                Label::new(
                    "On your router, forward UDP port 41641 to this computer, and keep it \
                     running. A phone on mobile data or a friend's laptop elsewhere then \
                     connects through it; their traffic uses this connection's upload \
                     bandwidth. Amber dots on the Devices page turning green show it works.",
                )
                .text_sm()
                .text_color(theme.muted_foreground),
            );
        }

        // Storage: where received files and identity data live.
        let storage = section("Storage")
            .child(
                Label::new(format!("received files: {}", paths::inbox_dir().display()))
                    .text_sm()
                    .truncate()
                    .text_color(theme.muted_foreground),
            )
            .child(
                Label::new(format!("device data: {}", paths::data_dir().display()))
                    .text_sm()
                    .truncate()
                    .text_color(theme.muted_foreground),
            );

        // Device ID (an npub underneath): a scannable QR plus a copy
        // action, never printed.
        let mut device_id = section("Device ID").child(
            Label::new("Share it with a device that joins this mesh, or with a member inviting this one.")
                .text_sm()
                .text_color(theme.muted_foreground),
        );
        if self.show_qr {
            // In a private mesh the QR also names its relays, so a new
            // device joins through them.
            if let Some(code) = session.pairing_uri.as_ref().or(session.my_npub.as_ref()) {
                if let Some(qr) = crate::views::join::qr_image(code) {
                    device_id = device_id.child(
                        div()
                            .p_3()
                            .border_1()
                            .border_color(theme.border)
                            .rounded(px(10.))
                            .bg(theme.secondary)
                            .w(px(202.))
                            .child(img(qr).size(px(176.))),
                    );
                }
            }
        }
        let copy_code = session.pairing_uri.clone().unwrap_or(npub);
        device_id = device_id.child(
            div()
                .flex()
                .flex_row()
                .gap_2()
                .child(
                    Button::new("toggle-qr")
                        .icon(IconName::QrCode)
                        .label(if self.show_qr { "Hide QR" } else { "Show QR" })
                        .compact()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_qr = !this.show_qr;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("copy-npub")
                        .icon(if self.copied { IconName::Check } else { IconName::Copy })
                        .label(if self.copied { "Copied" } else { "Copy" })
                        .compact()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy_code.clone()));
                            this.copied = true;
                            cx.notify();
                        })),
                ),
        );

        // Diagnostics: the numbers worth having when something is wrong.
        let keys = match crate::keystore::status() {
            crate::keystore::Storage::Sealed => "device keys encrypted, key in the system keyring".to_string(),
            crate::keystore::Storage::Plain(why) => format!("device keys stored unencrypted: {why}"),
            crate::keystore::Storage::Locked(why) => {
                format!("device keys are encrypted but the key is unavailable: {why}")
            }
        };
        let diagnostics = section("Diagnostics")
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
            .child(Label::new(keys).text_sm().text_color(theme.muted_foreground));

        // Danger section, set apart at the bottom. Disconnect is undone by
        // Connect, so it acts at once; Forget is permanent, so it asks.
        let danger = div().px_4().child(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .p_4()
                .rounded(px(10.))
                .border_1()
                .border_color(palette::danger().opacity(0.5))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .child(Label::new("Disconnect"))
                                .child(
                                    Label::new("Go offline. This computer stays in the mesh; Connect brings it back.")
                                        .text_sm()
                                        .text_color(theme.muted_foreground),
                                ),
                        )
                        .child(
                            Button::new("disconnect")
                                .label("Disconnect")
                                .compact()
                                .flex_shrink_0()
                                .disabled(forgetting || !connected)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.session.update(cx, |s, cx| s.disconnect(cx));
                                })),
                        ),
                )
                .child(div().h(px(1.)).bg(theme.border))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .child(Label::new("Forget mesh"))
                                .child(
                                    Label::new("Leave the mesh and delete this computer's identity and mesh data.")
                                        .text_sm()
                                        .text_color(theme.muted_foreground),
                                ),
                        )
                        .child(
                            // Works while stopped too: leaving runs the
                            // device just for that.
                            Button::new("forget")
                                .label(if forgetting { "Leaving the mesh..." } else { "Forget mesh..." })
                                .danger()
                                .compact()
                                .flex_shrink_0()
                                .disabled(forgetting)
                                .on_click(cx.listener(|this, _, window, cx| this.confirm_forget(window, cx))),
                        ),
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
                    .pb_6()
                    .max_w(px(560.))
                    // Offline, Settings is reachable from the sidebar; give
                    // the way back to the connect screen.
                    .children((session.mode == RunMode::Stopped).then(|| {
                        div().px_4().pt_3().child(
                            Button::new("back-connect")
                                .icon(IconName::ArrowLeft)
                                .label("Back to connect")
                                .ghost()
                                .compact()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.session.update(cx, |s, cx| s.set_tab(Tab::Devices, cx));
                                })),
                        )
                    }))
                    .child(status)
                    .child(name)
                    .child(network)
                    .child(appearance)
                    .child(device_id)
                    .child(storage)
                    .child(diagnostics)
                    .child(danger),
            )
            .into_any_element()
    }
}
