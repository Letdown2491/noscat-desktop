//! Settings tab: status, device name editing, diagnostics, disconnect behind
//! a two-step confirm.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::gpui::{
    div, rgb, AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window,
};

use crate::paths;
use crate::state::{RunMode, Session};

pub struct SettingsView {
    session: Entity<Session>,
    name_input: Entity<InputState>,
    /// The npub was copied to the clipboard (hint in diagnostics).
    copied: bool,
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
        let mode = match session.mode {
            RunMode::Stopped => "stopped",
            RunMode::Node => "node (chat, files, presence)",
            RunMode::Expose => "expose (serving a local service)",
        };
        let npub = session.my_npub.clone().unwrap_or_else(|| "-".to_string());
        let confirm = session.confirm_disconnect;
        let last_error = session.last_error.clone();

        // Diagnostics block: the Android settings screen's readouts.
        let mut status = div()
            .flex()
            .flex_col()
            .gap_1()
            .px_4()
            .child(Label::new("Status"))
            .child(
                Label::new(format!("mode: {mode}"))
                    .text_sm()
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        Label::new("npub:")
                            .text_sm()
                            .text_color(theme.muted_foreground),
                    )
                    // The npub text itself is clickable to copy, same action
                    // as the button beside it.
                    .child(
                        div()
                            .id("npub-text")
                            .cursor_pointer()
                            .on_click(cx.listener({
                                let npub = npub.clone();
                                move |this, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(npub.clone()));
                                    this.copied = true;
                                    cx.notify();
                                }
                            }))
                            .child(Label::new(npub.clone()).text_sm()),
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
                    )
                    .child(if self.copied {
                        Label::new("copied")
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .into_any_element()
                    } else {
                        div().into_any_element()
                    }),
            )
            .child(
                Label::new(format!(
                    "advert TTL: the client re-advertises while listening"
                ))
                .text_sm()
                .text_color(theme.muted_foreground),
            )
            .child(
                Label::new(format!("mesh dir: {}", paths::mesh_dir().display()))
                    .text_sm()
                    .text_color(theme.muted_foreground),
            )
            .child(
                Label::new(format!("inbox: {}", paths::inbox_dir().display()))
                    .text_sm()
                    .text_color(theme.muted_foreground),
            )
            .child(
                Label::new(format!(
                    "roster: {} devices, synced every 2 minutes",
                    session.roster.len()
                ))
                .text_sm()
                .text_color(theme.muted_foreground),
            );
        if let Some(error) = &last_error {
            status = status.child(
                Label::new(format!("last error: {error}"))
                    .text_sm()
                    .text_color(rgb(0xf87171)),
            );
        }

        // Device name editor: persists to settings.json; the client picks it
        // up on the next advert/start.
        let name = div()
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .child(Label::new("Device name"))
            .child(
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

        div()
            .flex()
            .flex_col()
            .size_full()
            .gap_3()
            .py_3()
            .id("settings-list")
            .overflow_y_scroll()
            .child(name)
            .child(status)
            .child(
                div().px_4().pt_2().child(
                    // Two-step confirm: the first click arms the button, the
                    // second disconnects (the Android app uses a dialog).
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
            )
            .into_any_element()
    }
}
