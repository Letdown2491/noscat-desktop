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
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::gpui::prelude::FluentBuilder as _;
use gpui_kit::gpui::{
    actions, div, px, AnyElement, App, AppContext as _, Context, Entity, FocusHandle,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_kit_assets::IconName;

use crate::palette;
use crate::state::{apply_theme, ErrorAction, RunMode, Session};
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
            for text in toasts {
                window.push_notification(Notification::success(text), cx);
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
            let my_npub = session.my_npub.as_ref();
            let reachable =
                session.roster.iter().filter(|d| d.connected || Some(&d.npub) == my_npub).count();
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
                (reachable, session.roster.len()),
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
            .bg(theme.secondary)
            .py_3()
            .child(
                div()
                    .px_4()
                    .pb_3()
                    .child(Label::new("noscat").text_size(px(17.))),
            );

        for (n, tab) in Tab::ALL.into_iter().enumerate() {
            if !connected && tab != Tab::Settings {
                continue;
            }
            let selected = active == tab && (connected || tab == Tab::Settings);
            let count = match tab {
                Tab::Chat => unread,
                Tab::Devices => requests,
                _ => 0,
            };
            let row = div()
                .id(SharedString::from(format!("nav-{}", tab.label())))
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_4()
                .py_1p5()
                .mx_2()
                .rounded(px(8.))
                .cursor_pointer()
                .text_color(if selected { theme.foreground } else { theme.muted_foreground })
                .when(selected, |s| s.bg(theme.accent))
                .hover(|s| s.bg(theme.accent).text_color(theme.foreground))
                .tooltip(move |window, cx| {
                    Tooltip::new(format!("{} (Ctrl+{})", tab.label(), n + 1)).build(window, cx)
                })
                .child(Icon::new(tab.icon()).with_size(px(16.)))
                .child(Label::new(tab.label()))
                .when(count > 0, |r| r.child(badge(count)))
                .on_click(cx.listener(move |this, _, _, cx| this.go(tab, cx)));
            sidebar = sidebar.child(row);
        }

        // Connection status in the sidebar footer, two lines so the device
        // name never truncates: the name on top, the state under it with
        // how much of the mesh is reachable. Clickable to Settings.
        let (reachable, known) = reach;
        let status_text = match (connected, starting) {
            (true, true) => "Restarting...".to_string(),
            (true, false) if known > 0 => format!("Connected {reachable} of {known}"),
            (true, false) => "Connected".to_string(),
            (false, true) => "Connecting...".to_string(),
            (false, false) => "Offline".to_string(),
        };
        let status = div()
            .id("status-to-settings")
            .mt_auto()
            .mx_2()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .p_2()
            .rounded(px(8.))
            .min_w_0()
            .cursor_pointer()
            .hover(|s| s.bg(theme.accent))
            .on_click(cx.listener(|this, _, _, cx| this.go(Tab::Settings, cx)))
            .child(div().flex_shrink_0().rounded_full().size(px(8.)).bg(if starting {
                palette::warning()
            } else if connected {
                palette::success()
            } else {
                palette::danger()
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(Label::new(footer_name).truncate())
                    .child(
                        Label::new(status_text)
                            .text_xs()
                            .truncate()
                            .text_color(theme.muted_foreground),
                    ),
            );
        sidebar = sidebar.child(status);

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
                                s.child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .px_4()
                                        .py_3()
                                        .border_b_1()
                                        .border_color(theme.border)
                                        .child(
                                            Label::new(if connected {
                                                active.label()
                                            } else {
                                                "Settings"
                                            })
                                            .text_size(px(16.)),
                                        ),
                                )
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
