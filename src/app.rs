//! The root view: a left navigation sidebar (the desktop shape of the
//! Android app's bottom tabs), the connection status in the sidebar footer,
//! a dismissible error strip with the action that fixes the error, the
//! active tab's view, and the app's keyboard shortcuts. Short confirmations
//! the session queues (`Session::toasts`) become window notifications here.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::label::Label;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::gpui::prelude::FluentBuilder as _;
use gpui_kit::gpui::{
    actions, div, px, AnyElement, App, AppContext as _, Context, Entity, FocusHandle, FontWeight,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_kit_assets::IconName;

use crate::palette;
use crate::state::{apply_theme, ErrorAction, RunMode, Session, Toast};
use crate::views::chat::ChatView;
use crate::views::devices::DevicesView;
use crate::views::files::FilesView;
use crate::views::join::JoinView;
use crate::views::services::ServicesView;
use crate::views::settings::SettingsView;

actions!(noscat, [ShowDevices, ShowChat, ShowFiles, ShowServices, ShowSettings, Dismiss]);

/// Ctrl+1..5 switch tabs (the sidebar order); Escape closes what is open
/// (the conversation, an inline editor).
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-1", ShowDevices, None),
        KeyBinding::new("ctrl-2", ShowChat, None),
        KeyBinding::new("ctrl-3", ShowFiles, None),
        KeyBinding::new("ctrl-4", ShowServices, None),
        KeyBinding::new("ctrl-5", ShowSettings, None),
        KeyBinding::new("escape", Dismiss, None),
    ]);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Devices,
    Chat,
    Files,
    Services,
    Settings,
}

impl Tab {
    pub const ALL: [Tab; 5] = [Tab::Devices, Tab::Chat, Tab::Files, Tab::Services, Tab::Settings];

    fn label(self) -> &'static str {
        match self {
            Tab::Devices => "Devices",
            Tab::Chat => "Chat",
            Tab::Files => "Files",
            Tab::Services => "Services",
            Tab::Settings => "Settings",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Tab::Devices => IconName::Users,
            Tab::Chat => IconName::MessageSquare,
            Tab::Files => IconName::FolderOpen,
            Tab::Services => IconName::Server,
            Tab::Settings => IconName::Settings,
        }
    }
}

pub struct NoscatApp {
    pub(crate) session: Entity<Session>,
    focus: FocusHandle,
    join: Entity<JoinView>,
    devices: Entity<DevicesView>,
    chat: Entity<ChatView>,
    files: Entity<FilesView>,
    services: Entity<ServicesView>,
    settings: Entity<SettingsView>,
}

/// A small count pill (unread messages, join requests).
fn badge(count: usize) -> impl IntoElement {
    div()
        .ml_auto()
        .rounded_full()
        .px_1p5()
        .min_w(px(18.))
        .text_center()
        .bg(palette::blue())
        .child(Label::new(count.to_string()).text_xs().text_color(palette::on_color()))
}

impl NoscatApp {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Re-render whenever session state changes, and turn queued
        // confirmations into notifications (they need the window).
        cx.observe_in(&session, window, |_, session, window, cx| {
            let toasts = session.update(cx, |s, _| std::mem::take(&mut s.toasts));
            for toast in toasts {
                let note = match toast {
                    Toast::Done(text) => Notification::success(text),
                    // A failed send waits for the user: Try again sends
                    // the same file (the receiver resumes what it has).
                    Toast::SendFailed { peer, name, path } => {
                        let who = session.read(cx).peer_name(&peer);
                        let session = session.clone();
                        Notification::error(format!("Couldn't send {name} to {who}")).action(
                            move |_, _, cx| {
                                let note = cx.entity();
                                let session = session.clone();
                                let (peer, path) = (peer.clone(), path.clone());
                                Button::new("try-again").label("Try again").compact().on_click(
                                    move |_, window, cx| {
                                        session.update(cx, |s, cx| s.send_file(&peer, path.clone(), cx));
                                        note.update(cx, |n, cx| n.dismiss(window, cx));
                                    },
                                )
                            },
                        )
                    }
                };
                window.push_notification(note, cx);
            }
            cx.notify();
        })
        .detach();
        // Focus decides whether a message for the open conversation still
        // notifies.
        cx.observe_window_activation(window, |this, window, cx| {
            let active = window.is_window_active();
            this.session.update(cx, |s, _| s.window_active = active);
        })
        .detach();
        // The shortcuts dispatch through the focused element's ancestors;
        // focusing the root keeps them working before anything is clicked.
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let join = cx.new(|cx| JoinView::new(session.clone(), window, cx));
        let devices = cx.new(|cx| DevicesView::new(session.clone(), window, cx));
        let chat = cx.new(|cx| ChatView::new(session.clone(), window, cx));
        let files = cx.new(|cx| FilesView::new(session.clone(), window, cx));
        let services = cx.new(|cx| ServicesView::new(session.clone(), window, cx));
        let settings = cx.new(|cx| SettingsView::new(session.clone(), window, cx));
        Self {
            session,
            focus,
            join,
            devices,
            chat,
            files,
            services,
            settings,
        }
    }

    fn render_active(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.session.read(cx).tab {
            Tab::Devices => self.devices.clone().into_any_element(),
            Tab::Chat => self.chat.clone().into_any_element(),
            Tab::Files => self.files.clone().into_any_element(),
            Tab::Services => self.services.clone().into_any_element(),
            Tab::Settings => self.settings.clone().into_any_element(),
        }
    }

    /// A page's header: its title, the context that belongs to it, and
    /// the page's own action.
    fn page_header(&self, tab: Tab, others: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let action = match tab {
            Tab::Services => Some(
                Button::new("header-new-forward")
                    .icon(IconName::Plus)
                    .label("New forward")
                    .compact()
                    .disabled(others == 0)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.services.update(cx, |v, cx| v.open_new_forward(window, cx));
                    })),
            ),
            _ => None,
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .h(px(56.))
            .flex_shrink_0()
            .px_5()
            .border_b_1()
            .border_color(theme.border)
            .child(crate::ui::page_title(tab.label()))
            .child(div().flex_1())
            .children(action)
    }

    /// Switch tabs from a shortcut; offline, only Settings is a tab.
    fn go(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.session.update(cx, |s, cx| {
            let connected = s.mode != RunMode::Stopped || s.restarting;
            if connected || tab == Tab::Settings {
                s.set_tab(tab, cx);
            }
        });
    }

    /// Escape: close the open conversation or the Devices page's inline
    /// editors.
    fn dismiss(&mut self, cx: &mut Context<Self>) {
        let tab = self.session.read(cx).tab;
        match tab {
            Tab::Chat => self.session.update(cx, |s, cx| {
                s.active_chat = None;
                cx.notify();
            }),
            Tab::Devices => self.devices.update(cx, |d, cx| d.close_editors(cx)),
            _ => {}
        }
    }
}

impl Render for NoscatApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Read everything the shell needs up front so the render body can
        // freely mutate state (the system-theme resync below does).
        let (connected, error, error_action, active, unread, requests, theme_pref, starting, footer_name, reach) = {
            let session = self.session.read(cx);
            (
                // A restart (reachable toggled, expose started or
                // stopped) keeps the tabs on screen.
                session.mode != RunMode::Stopped || session.restarting,
                session.last_error.clone(),
                // The fix applies only while its own error is showing.
                session
                    .error_action
                    .as_ref()
                    .filter(|(_, text)| Some(text) == session.last_error.as_ref())
                    .map(|(action, _)| *action),
                session.tab,
                session.unread.values().sum::<usize>(),
                session.join_requests.len(),
                session.settings.theme,
                session.starting,
                // The footer names the mesh; before the first sync of a
                // new install, this device's name stands in.
                session
                    .settings
                    .mesh_name
                    .clone()
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| session.settings.device_name.clone()),
                // Other devices connected, of other devices.
                session.reach(),
            )
        };
        let theme = cx.theme().clone();

        // Follow the system theme while the preference is System: GPUI
        // delivers appearance changes by re-rendering, so re-syncing here
        // catches a desktop theme flip (the guard keeps it to one pass).
        if theme_pref == crate::paths::ThemePref::System {
            let system_dark = matches!(
                window.appearance(),
                gpui_kit::gpui::WindowAppearance::Dark
                    | gpui_kit::gpui::WindowAppearance::VibrantDark
            );
            if theme.mode.is_dark() != system_dark {
                apply_theme(crate::paths::ThemePref::System, Some(window), cx);
            }
        }

        // Sidebar: brand, one nav row per tab (icon, label, a count badge on
        // Chat and Devices), and the connection status as the footer.
        // Offline, only Settings is a tab: the others need a connection,
        // and greyed rows explain nothing.
        let mut sidebar = div()
            .flex()
            .flex_col()
            .w(px(200.))
            .flex_shrink_0()
            .h_full()
            .border_r_1()
            .border_color(theme.border)
            .bg(theme.secondary);

        // Header: the mesh this computer is in, like a workspace switcher:
        // the noscat mark, the mesh name, and its state under it. The
        // same height as the page headers so their bottom borders line up.
        // Clicking it opens Settings, where the connection lives.
        let (reachable, known) = reach;
        let (dot, status_text) = match (connected, starting) {
            (true, true) => (palette::warning(), "Restarting...".to_string()),
            (true, false) if known > 0 => (palette::success(), format!("Connected {reachable} of {known}")),
            // Alone in the mesh: nothing to count.
            (true, false) => (palette::success(), "Connected".to_string()),
            (false, true) => (palette::warning(), "Connecting...".to_string()),
            (false, false) => (palette::danger(), "Offline".to_string()),
        };
        sidebar = sidebar.child(
            div()
                .id("mesh-header")
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .h(px(56.))
                .flex_shrink_0()
                .px_4()
                .border_b_1()
                .border_color(theme.border)
                .cursor_pointer()
                .hover(|s| s.bg(theme.foreground.opacity(0.04)))
                // The ring's colour is the state; hover says it in words
                // (colour alone does not reach everyone).
                .tooltip(move |window, cx| Tooltip::new(status_text.clone()).build(window, cx))
                .on_click(cx.listener(|this, _, _, cx| this.go(Tab::Settings, cx)))
                // A halo around the mark: green connected, amber connecting,
                // red offline. Shaped like the mark's rounded square.
                .child(
                    div()
                        .flex_shrink_0()
                        .p(px(2.))
                        .rounded(px(10.))
                        .border_2()
                        .border_color(dot)
                        .child(crate::ui::logo(26.)),
                )
                .child(
                    Label::new(footer_name.clone())
                        .font_weight(FontWeight::SEMIBOLD)
                        .truncate(),
                ),
        );

        // One row style for every sidebar entry. Selected: a filled pill
        // and full-strength text; the rest muted until hovered.
        let nav_row = |id: SharedString,
                       icon: IconName,
                       label: &'static str,
                       selected: bool,
                       count: usize,
                       tip: Option<String>| {
            div()
                .id(id)
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .h(px(34.))
                .px_3()
                .mx_2()
                .rounded(px(8.))
                .cursor_pointer()
                .text_color(if selected { theme.foreground } else { theme.muted_foreground })
                .when(selected, |s| s.bg(theme.foreground.opacity(0.09)))
                .hover(|s| s.bg(theme.foreground.opacity(0.06)).text_color(theme.foreground))
                .when_some(tip, |r, tip| {
                    r.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                })
                .child(Icon::new(icon).with_size(px(16.)))
                .child(
                    Label::new(label)
                        .text_sm()
                        .text_color(if selected { theme.foreground } else { theme.muted_foreground })
                        .when(selected, |l| l.font_weight(FontWeight::MEDIUM)),
                )
                .when(count > 0, |r| r.child(div().ml_auto().child(badge(count))))
        };

        // The pages, then (pinned to the bottom, where desktop apps keep
        // them) Invite a device and Settings. Offline, only Settings: the
        // others need a connection.
        let mut pages = div().flex().flex_col().gap_0p5().pt_3();
        for (n, tab) in Tab::ALL.into_iter().enumerate() {
            if tab == Tab::Settings || !connected {
                continue;
            }
            let count = match tab {
                Tab::Chat => unread,
                Tab::Devices => requests,
                _ => 0,
            };
            pages = pages.child(
                nav_row(
                    SharedString::from(format!("nav-{}", tab.label())),
                    tab.icon(),
                    tab.label(),
                    active == tab,
                    count,
                    Some(format!("Ctrl+{}", n + 1)),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.go(tab, cx))),
            );
        }
        let settings_n = Tab::ALL.iter().position(|t| *t == Tab::Settings).unwrap_or(4) + 1;
        let bottom = div()
            .mt_auto()
            .flex()
            .flex_col()
            .gap_0p5()
            .pb_3()
            .when(connected, |b| {
                b.child(
                    nav_row("nav-invite".into(), IconName::UserPlus, "Invite a device", false, 0, None)
                        .on_click(cx.listener(|this, _, window, cx| {
                            crate::views::devices::open_pairing_dialog(&this.session, window, cx);
                        })),
                )
            })
            .child(
                nav_row(
                    "nav-Settings".into(),
                    Tab::Settings.icon(),
                    "Settings",
                    active == Tab::Settings || !connected,
                    0,
                    Some(format!("Ctrl+{settings_n}")),
                )
                .on_click(cx.listener(|this, _, _, cx| this.go(Tab::Settings, cx))),
            );
        sidebar = sidebar.child(pages).child(bottom);

        // Errors render once, in a strip across the bottom of the window:
        // the text (all of it on hover), the action that fixes it when
        // there is one, and dismiss.
        let error_footer = error.map(|e| {
            let full = e.clone();
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_4()
                .py_1p5()
                .bg(theme.secondary)
                .border_t_1()
                .border_color(theme.border)
                .child(
                    Icon::new(IconName::TriangleAlert)
                        .with_size(px(15.))
                        .text_color(palette::warning()),
                )
                .child(
                    div()
                        .id("error-text")
                        .flex_1()
                        .min_w_0()
                        .tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx))
                        .child(Label::new(e).text_sm().truncate().text_color(theme.muted_foreground)),
                )
                .children(error_action.map(|action| {
                    let (label, id) = match action {
                        ErrorAction::Connect => (if starting { "Connecting..." } else { "Connect" }, "error-connect"),
                        ErrorAction::OpenSettings => ("Open Settings", "error-settings"),
                    };
                    Button::new(id)
                        .label(label)
                        .compact()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.session.update(cx, |s, cx| {
                                match action {
                                    ErrorAction::Connect => s.start_node(cx),
                                    ErrorAction::OpenSettings => s.set_tab(Tab::Settings, cx),
                                }
                                s.clear_error(cx);
                            });
                        }))
                }))
                .child(
                    Button::new("dismiss-error")
                        .icon(IconName::X)
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.session.update(cx, |s, cx| s.clear_error(cx));
                        })),
                )
                .into_any_element()
        });

        div()
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &ShowDevices, _, cx| this.go(Tab::Devices, cx)))
            .on_action(cx.listener(|this, _: &ShowChat, _, cx| this.go(Tab::Chat, cx)))
            .on_action(cx.listener(|this, _: &ShowFiles, _, cx| this.go(Tab::Files, cx)))
            .on_action(cx.listener(|this, _: &ShowServices, _, cx| this.go(Tab::Services, cx)))
            .on_action(cx.listener(|this, _: &ShowSettings, _, cx| this.go(Tab::Settings, cx)))
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| this.dismiss(cx)))
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .child(sidebar)
                    // Content: a page header (screen title), then the active
                    // tab. Offline shows the join flow, which has its own
                    // hero and needs no header, unless the user opened
                    // Settings.
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .when(connected || active == Tab::Settings, |s| {
                                s.child(self.page_header(if connected { active } else { Tab::Settings }, known, cx))
                            })
                            .child(
                                div().flex_1().min_h_0().child(if connected {
                                    self.render_active(cx)
                                } else if active == Tab::Settings {
                                    self.settings.clone().into_any_element()
                                } else {
                                    self.join.clone().into_any_element()
                                }),
                            ),
                    ),
            )
            .children(error_footer)
    }
}
