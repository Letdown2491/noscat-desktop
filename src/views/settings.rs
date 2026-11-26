//! Settings tab, grouped into cards: This computer (name, pairing code),
//! Network (reachability, port, the mesh's STUN servers, how the keys are
//! kept), Access (who may do what to this computer), Appearance,
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
use noscat_client::{AccessGroup, AccessRule, AccessRules};

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
    stun_input: Entity<InputState>,
    /// The STUN list the input was last filled from.
    stun_seen: Option<Vec<String>>,
    /// Whose access rules are open for editing: `Some(None)` is everyone.
    access_open: Option<Option<String>>,
}

/// "Send allowed, services allowed, control asks".
fn summary(r: &AccessRules) -> String {
    let word = |rule| crate::state::control::rule_word(rule);
    format!("Send {}, services {}, control {}", word(r.send), word(r.services), word(r.control))
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
        let stun_input = cx.new(|cx| InputState::new(window, cx).placeholder("None set (ip:port, ...)"));
        cx.subscribe_in(&stun_input, window, |this, _, ev: &InputEvent, _, cx| match ev {
            InputEvent::Change => cx.notify(),
            InputEvent::PressEnter { .. } => this.save_stun(cx),
            _ => {}
        })
        .detach();
        Self {
            session,
            name_input,
            show_reach_help: false,
            saved_at: None,
            stun_input,
            stun_seen: None,
            access_open: None,
        }
    }

    fn typed_stun(&self, cx: &Context<Self>) -> Vec<String> {
        self.stun_input
            .read(cx)
            .value()
            .split([',', ' '])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect()
    }

    fn save_stun(&mut self, cx: &mut Context<Self>) {
        let list = self.typed_stun(cx);
        if Some(&list) == self.stun_seen.as_ref() {
            return;
        }
        self.session.update(cx, |s, cx| s.set_mesh_stun(list, cx));
    }

    /// One group's rule as a segmented control.
    fn rule_picker(&self, who: Option<String>, group: AccessGroup, current: AccessRule, cx: &mut Context<Self>) -> Div {
        let mut options = vec![(AccessRule::Allow, "Allow")];
        if group == AccessGroup::Control {
            options.push((AccessRule::Ask, "Ask"));
        }
        options.push((AccessRule::Deny, "Block"));
        let key = who.clone().unwrap_or_else(|| "everyone".into());
        div().flex().flex_row().gap_1().children(options.into_iter().map(|(rule, label)| {
            let who = who.clone();
            let b = Button::new(gpui_kit::gpui::SharedString::from(format!("rule-{key}-{}-{}", group.name(), rule.name())))
                .label(label)
                .compact();
            let b = if rule == current { b.primary() } else { b.ghost() };
            b.on_click(cx.listener(move |this, _, _, cx| {
                let who = who.clone();
                this.session.update(cx, |s, cx| s.set_access(who, group, Some(rule), cx));
            }))
        }))
    }

    /// The Access card: everyone, then each member, then recent sessions.
    fn access_card(&self, border: Hsla, surface: Hsla, muted: Hsla, cx: &mut Context<Self>) -> Div {
        let session = self.session.read(cx);
        let defaults = session.control.defaults;
        let me = session.my_npub.clone();
        let members: Vec<(String, String, Option<AccessRules>, bool)> = session
            .roster
            .iter()
            .filter(|d| Some(&d.npub) != me.as_ref())
            .map(|d| {
                (d.npub.clone(), d.display_name(), session.control.rules.get(&d.npub).copied(), session.control.custom.contains(&d.npub))
            })
            .collect();
        let recent: Vec<(String, u64, u64)> = session
            .control
            .recent
            .iter()
            .map(|e| (session.peer_name(&e.peer), e.secs, e.ended.elapsed().as_secs()))
            .collect();
        let connected = session.mode != RunMode::Stopped;
        let mut rows: Vec<AnyElement> = Vec::new();
        if !connected || defaults.is_none() {
            rows.push(
                row("Access", Some("Connect to see and change who may do what here.".into()), div(), muted).into_any_element(),
            );
            return card("Access", rows, border, surface, muted);
        }
        let mut entries: Vec<(Option<String>, String, Option<AccessRules>, bool)> =
            vec![(None, "Everyone".to_string(), defaults, false)];
        entries.extend(members.into_iter().map(|(n, name, r, c)| (Some(n), name, r, c)));
        for (who, name, rules, custom) in entries {
            let Some(rules) = rules else { continue };
            let open = self.access_open.as_ref() == Some(&who);
            let detail = if who.is_none() {
                format!("{}. Applies to devices without rules of their own.", summary(&rules))
            } else if custom {
                format!("{} (own rules)", summary(&rules))
            } else {
                summary(&rules)
            };
            let key = who.clone().unwrap_or_else(|| "everyone".into());
            let toggle = Button::new(gpui_kit::gpui::SharedString::from(format!("access-edit-{key}")))
                .label(if open { "Done" } else { "Edit" })
                .compact()
                .ghost()
                .on_click(cx.listener({
                    let who = who.clone();
                    move |this, _, _, cx| {
                        this.access_open = if this.access_open.as_ref() == Some(&who) { None } else { Some(who.clone()) };
                        cx.notify();
                    }
                }));
            let mut block = div().flex().flex_col().child(row(&name, Some(detail), toggle, muted));
            if open {
                let mut editor = div().flex().flex_col().gap_2().px_4().pb_3();
                for group in AccessGroup::ALL {
                    editor = editor.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_3()
                            .child(Label::new(crate::state::control::group_label(group)).text_sm().flex_1())
                            .child(self.rule_picker(who.clone(), group, crate::state::control::rule_of(&rules, group), cx)),
                    );
                }
                if let (Some(n), true) = (who.clone(), custom) {
                    editor = editor.child(
                        div().child(
                            Button::new(gpui_kit::gpui::SharedString::from(format!("access-reset-{n}")))
                                .label("Use the defaults")
                                .compact()
                                .ghost()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let n = n.clone();
                                    this.session.update(cx, |s, cx| s.reset_access(n, cx));
                                })),
                        ),
                    );
                }
                block = block.child(editor);
            }
            rows.push(block.into_any_element());
        }
        let recent_text = if recent.is_empty() {
            "None since this app started.".to_string()
        } else {
            recent
                .iter()
                .map(|(who, secs, ago)| format!("{who}, {}, {} ago", crate::state::control::span(*secs), crate::state::control::span(*ago)))
                .collect::<Vec<_>>()
                .join("; ")
        };
        rows.push(row("Recent terminal sessions here", Some(recent_text), div(), muted).into_any_element());
        card("Access", rows, border, surface, muted)
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

        // Fill the STUN field when the list arrives or changes elsewhere.
        let stun = self.session.read(cx).control.stun.clone();
        if self.stun_seen.as_ref() != Some(&stun) && self.session.read(cx).control.defaults.is_some() {
            let text = stun.join(", ");
            self.stun_input.update(cx, |i, cx| i.set_value(text, window, cx));
            self.stun_seen = Some(stun);
        }
        let stun_typed = self.typed_stun(cx);
        let theme = cx.theme().clone();
        let (border, surface, muted) = (theme.border, theme.secondary, theme.muted_foreground);
        let access = self.access_card(border, surface, muted, cx);
        let session = self.session.read(cx);
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
        let stun_row = div()
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .py_3()
            .child(Label::new("STUN servers"))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().max_w(px(360.)).child(Input::new(&self.stun_input).disabled(!connected)))
                    .child(
                        Button::new("save-stun")
                            .label("Save")
                            .compact()
                            .disabled(!connected || Some(&stun_typed) == self.stun_seen.as_ref())
                            .on_click(cx.listener(|this, _, _, cx| this.save_stun(cx))),
                    ),
            )
            .child(
                Label::new(
                    "How the mesh's devices learn their public address, as ip:port, separated by commas. \
                     Shared with every member. Empty: public servers, or none on a private mesh.",
                )
                .text_sm()
                .text_color(muted),
            );
        let network = card(
            "Network",
            vec![
                reach_row.into_any_element(),
                port_row.into_any_element(),
                stun_row.into_any_element(),
                keys_row.into_any_element(),
            ],
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
                    .child(access)
                    .child(appearance)
                    .child(storage)
                    .child(danger),
            )
            .into_any_element()
    }
}
