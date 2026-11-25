//! Settings tab, grouped into cards: This computer (name, pairing code),
//! Network (reachability, port, how the keys are kept), Appearance,
//! Storage (folders, openable), and the danger zone: Disconnect
//! (reversible, so immediate) and Forget mesh (behind a confirm dialog).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, WindowExt as _};
use gpui_kit::gpui::prelude::FluentBuilder as _;
use gpui_kit::gpui::{
    div, px, AnyElement, AppContext as _, Context, Div, Entity, Hsla, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_kit_assets::IconName;

use crate::app::Tab;
use crate::palette;
use crate::paths;
use crate::state::{RunMode, Session};

/// How long "Saved" shows after a rename.
const SAVED_FOR: Duration = Duration::from_secs(2);

/// Longest path shown whole; longer ones lose their middle.
const PATH_CHARS: usize = 44;

pub struct SettingsView {
    session: Entity<Session>,
    name_input: Entity<InputState>,
    /// The reachability setup steps are open.
    show_reach_help: bool,
    /// When the name was last saved, for the "Saved" confirmation.
    saved_at: Option<Instant>,
}

/// "/var/home/me/.local/share/noscat-desktop" as
/// "/var/home/me/.lo...noscat-desktop": the start says where, the end
/// says what.
fn middle(path: &std::path::Path) -> String {
    let text = path.display().to_string();
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= PATH_CHARS {
        return text;
    }
    let keep = (PATH_CHARS - 3) / 2;
    let head: String = chars[..keep].iter().collect();
    let tail: String = chars[chars.len() - keep..].iter().collect();
    format!("{head}...{tail}")
}

/// A card: its label above, then rows separated by hairlines.
fn card(title: &str, rows: Vec<AnyElement>, border: Hsla, surface: Hsla, muted: Hsla) -> Div {
    let mut body = div()
        .flex()
        .flex_col()
        .rounded(px(10.))
        .border_1()
        .border_color(border)
        .bg(surface);
    let count = rows.len();
    for (i, row) in rows.into_iter().enumerate() {
        body = body.child(row);
        if i + 1 < count {
            body = body.child(div().h(px(1.)).mx_4().bg(border));
        }
    }
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().px_1().child(crate::ui::section_label(title.to_uppercase(), muted)))
        .child(body)
}

/// A row: title and description on the left, its control on the right.
fn row(title: &str, description: Option<String>, control: impl IntoElement, muted: Hsla) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_4()
        .px_4()
        .py_3()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .flex_1()
                .min_w_0()
                .child(Label::new(title.to_string()))
                .children(description.map(|d| Label::new(d).text_sm().text_color(muted))),
        )
        .child(div().flex_shrink_0().child(control))
}

/// Open a folder in the file manager without blocking the UI.
fn open_folder(cx: &mut Context<SettingsView>, dir: PathBuf) {
    cx.spawn(async move |_, cx| {
        cx.background_executor()
            .spawn(async move {
                let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
            })
            .await;
    })
    .detach();
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
        let (border, surface, muted) = (theme.border, theme.secondary, theme.muted_foreground);
        let connected = session.mode != RunMode::Stopped;
        let forgetting = session.forgetting;
        let stopped = session.mode == RunMode::Stopped;

        // This computer: its name, and the code that lets a device join.
        let typed = self.name_input.read(cx).value().trim().to_string();
        let changed = !typed.is_empty() && typed != session.settings.device_name;
        let saved = self.saved_at.is_some_and(|t| t.elapsed() < SAVED_FOR);
        let name_row = div()
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .py_3()
            .child(Label::new("Name"))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().max_w(px(280.)).child(Input::new(&self.name_input)))
                    .child(
                        Button::new("save-name")
                            .label("Save")
                            .primary()
                            .compact()
                            .disabled(!changed)
                            .on_click(cx.listener(|this, _, _, cx| this.save_name(cx))),
                    )
                    .when(saved && !changed, |r| {
                        r.child(
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
            )
            .child(Label::new("How other devices see this computer.").text_sm().text_color(muted));
        let pairing_row = row(
            "Pairing code",
            Some("A new device scans or pastes it to ask to join.".into()),
            Button::new("show-pairing")
                .icon(IconName::QrCode)
                .label("Show pairing code")
                .compact()
                .disabled(session.my_npub.is_none())
                .on_click(cx.listener(|this, _, window, cx| {
                    crate::views::devices::open_pairing_dialog(&this.session, window, cx);
                })),
            muted,
        );
        let this_computer = card(
            "This computer",
            vec![name_row.into_any_element(), pairing_row.into_any_element()],
            border,
            surface,
            muted,
        );

        // Network: reachability (setup behind a disclosure), the port, and
        // how the device keys are kept.
        let reachable_on = session.settings.reachable;
        let mut reach_row = div().flex().flex_col().child(row(
            "Reachable from the internet",
            Some("Lets devices on other networks connect through this computer. One per mesh is enough.".into()),
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
            muted,
        ));
        reach_row = reach_row.child(
            div()
                .px_4()
                .pb_3()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .id("reach-help")
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .cursor_pointer()
                        .text_color(muted)
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
                )
                .when(self.show_reach_help, |d| {
                    d.child(
                        Label::new(
                            "On your router, forward UDP port 41641 to this computer, and keep it \
                             running. A phone on mobile data or a laptop elsewhere then connects \
                             through it, using this connection's upload bandwidth. Amber dots on \
                             the Devices page turning green show it works.",
                        )
                        .text_sm()
                        .text_color(muted),
                    )
                }),
        );
        let port = match (connected, session.listen_port) {
            (true, Some(port)) => format!("UDP {port}"),
            (true, None) => "Not bound yet".into(),
            (false, _) => "Not running".into(),
        };
        let port_row = row("Listening port", None, Label::new(port).text_sm().text_color(muted), muted);
        let (keys, keys_detail) = match crate::keystore::status() {
            crate::keystore::Storage::Sealed => ("Encrypted".to_string(), "The key is in the system keyring.".to_string()),
            crate::keystore::Storage::Plain(why) => ("Not encrypted".to_string(), why.to_string()),
            crate::keystore::Storage::Locked(why) => ("Key unavailable".to_string(), why.to_string()),
        };
        let keys_row = row(
            "Device keys",
            Some(keys_detail),
            Label::new(keys).text_sm().text_color(muted),
            muted,
        );
        let network = card(
            "Network",
            vec![reach_row.into_any_element(), port_row.into_any_element(), keys_row.into_any_element()],
            border,
            surface,
            muted,
        );

        // Appearance: system / light / dark, segmented.
        let theme_pref = session.settings.theme;
        let segments = div().flex().flex_row().gap_1().children({
            let mut buttons = Vec::new();
            for (pref, label, id) in [
                (paths::ThemePref::System, "System", "theme-system"),
                (paths::ThemePref::Light, "Light", "theme-light"),
                (paths::ThemePref::Dark, "Dark", "theme-dark"),
            ] {
                let button = Button::new(id).label(label).compact();
                let button = if theme_pref == pref { button.primary() } else { button.ghost() };
                buttons.push(button.on_click(cx.listener(move |this, _, window, cx| {
                    this.session.update(cx, |s, cx| s.set_theme(pref, cx));
                    crate::state::apply_theme(pref, Some(window), cx);
                    cx.notify();
                })));
            }
            buttons
        });
        let appearance = card(
            "Appearance",
            vec![row("Theme", None, segments, muted).into_any_element()],
            border,
            surface,
            muted,
        );

        // Storage: where things live, one click away.
        let folder_row = |id: &'static str, title: &str, dir: PathBuf, cx: &mut Context<Self>| {
            row(
                title,
                Some(middle(&dir)),
                Button::new(id)
                    .icon(IconName::FolderOpen)
                    .label("Open")
                    .compact()
                    .on_click(cx.listener(move |_, _, _, cx| open_folder(cx, dir.clone()))),
                muted,
            )
            .into_any_element()
        };
        let storage = card(
            "Storage",
            vec![
                folder_row("open-inbox", "Received files", paths::inbox_dir(), cx),
                folder_row("open-data", "Device data", paths::data_dir(), cx),
            ],
            border,
            surface,
            muted,
        );

        // Danger zone. Disconnect is undone by Connect, so it acts at once;
        // Forget is permanent, so it asks.
        let danger_rows = vec![
            row(
                "Disconnect",
                Some("Go offline. This computer stays in the mesh; Connect brings it back.".into()),
                Button::new("disconnect")
                    .label("Disconnect")
                    .compact()
                    .disabled(forgetting || !connected)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |s, cx| s.disconnect(cx));
                    })),
                muted,
            )
            .into_any_element(),
            row(
                "Forget mesh",
                Some("Leave the mesh and delete this computer's identity and mesh data.".into()),
                // Works while stopped too: leaving runs the device just for
                // that.
                Button::new("forget")
                    .label(if forgetting { "Leaving..." } else { "Forget mesh..." })
                    .danger()
                    .compact()
                    .disabled(forgetting)
                    .on_click(cx.listener(|this, _, window, cx| this.confirm_forget(window, cx))),
                muted,
            )
            .into_any_element(),
        ];
        let danger = card("Danger zone", danger_rows, palette::danger().opacity(0.45), surface, muted);

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
                    .gap_6()
                    .px_5()
                    .py_5()
                    .max_w(px(640.))
                    // Offline, Settings is reachable from the sidebar; give
                    // the way back to the connect screen.
                    .children(stopped.then(|| {
                        div().child(
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
                    .child(this_computer)
                    .child(network)
                    .child(appearance)
                    .child(storage)
                    .child(danger),
            )
            .into_any_element()
    }
}
