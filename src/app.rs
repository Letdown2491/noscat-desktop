//! The root view: a left navigation sidebar (the desktop shape of the
//! Android app's bottom tabs), the connection status in the sidebar footer,
//! a dismissible error banner over the content, and the active tab's view.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::label::Label;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::gpui::prelude::FluentBuilder as _;
use gpui_kit::gpui::{
    div, px, rgb, AnyElement, AppContext as _, Context, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window,
};
use gpui_kit_assets::IconName;

use crate::state::{apply_theme, RunMode, Session};
use crate::views::chat::ChatView;
use crate::views::devices::DevicesView;
use crate::views::files::FilesView;
use crate::views::join::JoinView;
use crate::views::settings::SettingsView;
use crate::views::shares::SharesView;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Devices,
    Chat,
    Files,
    Shares,
    Settings,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Tab::Devices => "Devices",
            Tab::Chat => "Chat",
            Tab::Files => "Files",
            Tab::Shares => "Shares",
            Tab::Settings => "Settings",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Tab::Devices => IconName::Users,
            Tab::Chat => IconName::MessageSquare,
            Tab::Files => IconName::FolderOpen,
            Tab::Shares => IconName::Share2,
            Tab::Settings => IconName::Settings,
        }
    }
}

pub struct NoscatApp {
    pub(crate) session: Entity<Session>,
    join: Entity<JoinView>,
    devices: Entity<DevicesView>,
    chat: Entity<ChatView>,
    files: Entity<FilesView>,
    shares: Entity<SharesView>,
    settings: Entity<SettingsView>,
}

impl NoscatApp {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Re-render whenever session state changes.
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        let join = cx.new(|cx| JoinView::new(session.clone(), window, cx));
        let devices = cx.new(|cx| DevicesView::new(session.clone(), window, cx));
        let chat = cx.new(|cx| ChatView::new(session.clone(), window, cx));
        let files = cx.new(|cx| FilesView::new(session.clone(), window, cx));
        let shares = cx.new(|cx| SharesView::new(session.clone(), window, cx));
        let settings = cx.new(|cx| SettingsView::new(session.clone(), window, cx));
        Self {
            session,
            join,
            devices,
            chat,
            files,
            shares,
            settings,
        }
    }

    fn render_active(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.session.read(cx).tab {
            Tab::Devices => self.devices.clone().into_any_element(),
            Tab::Chat => self.chat.clone().into_any_element(),
            Tab::Files => self.files.clone().into_any_element(),
            Tab::Shares => self.shares.clone().into_any_element(),
            Tab::Settings => self.settings.clone().into_any_element(),
        }
    }
}

impl Render for NoscatApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Read everything the shell needs up front so the render body can
        // freely mutate state (the system-theme resync below does).
        let (connected, error, active, unread, theme_pref, starting, device_name) = {
            let session = self.session.read(cx);
            (
                session.mode != RunMode::Stopped,
                session.last_error.clone(),
                session.tab,
                session.unread.values().sum::<usize>(),
                session.settings.theme,
                session.starting,
                session.settings.device_name.clone(),
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

        // Sidebar: brand, one nav row per tab (icon + label, the unread
        // badge on Chat), and the connection status as the footer. Nav
        // rows are full-width buttons - standard desktop navigation.
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

        for tab in [
            Tab::Devices,
            Tab::Chat,
            Tab::Files,
            Tab::Shares,
            Tab::Settings,
        ] {
            let enabled = connected || tab == Tab::Settings;
            let selected = connected && active == tab;
            let is_chat_unread = tab == Tab::Chat && unread > 0;
            let mut row = div()
                .id(SharedString::from(format!("nav-{}", tab.label())))
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_4()
                .py_1p5()
                .mx_2()
                .rounded(px(8.))
                .text_color(if selected {
                    theme.foreground
                } else {
                    theme.muted_foreground
                })
                .when(selected, |s| s.bg(theme.accent))
                .when(enabled, |s| {
                    s.cursor_pointer().hover(|s| {
                        s.bg(theme.accent).text_color(theme.foreground)
                    })
                })
                .child(
                    Icon::new(tab.icon())
                        .with_size(px(16.))
                        .text_color(if selected {
                            theme.foreground
                        } else {
                            theme.muted_foreground
                        }),
                )
                .child(Label::new(tab.label()));
            if is_chat_unread {
                row = row.child(
                    div()
                        .ml_auto()
                        .rounded_full()
                        .px_1p5()
                        .min_w(px(18.))
                        .text_center()
                        .bg(rgb(0x2563eb))
                        .child(
                            Label::new(unread.to_string())
                                .text_xs()
                                .text_color(rgb(0xffffff)),
                        ),
                );
            }
            let row = if enabled {
                row.on_click(cx.listener(move |this, _, _, cx| {
                    this.session.update(cx, |s, cx| s.set_tab(tab, cx));
                }))
            } else {
                row
            };
            sidebar = sidebar.child(row);
        }

        // Connection status in the sidebar footer: dot, device name,
        // connection state. Clickable to Settings once connected, where
        // disconnect lives. Names the device, never the npub.
        let status_text = match (connected, starting) {
            (true, _) => format!("connected - {device_name}"),
            (false, true) => "connecting...".to_string(),
            (false, false) => "offline".to_string(),
        };
        let status_inner = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .p_2()
            .rounded(px(8.))
            .min_w_0()
            .child(
                div().flex_shrink_0().rounded_full().size(px(8.)).bg(if connected {
                    rgb(0x22c55e)
                } else if starting {
                    rgb(0xeab308)
                } else {
                    rgb(0xef4444)
                }),
            )
            .child(
                Label::new(status_text)
                    .text_sm()
                    .truncate()
                    .text_color(theme.muted_foreground),
            );
        let status = div().mt_auto().mx_2().child(if connected {
            status_inner
                .id("status-to-settings")
                .cursor_pointer()
                .hover(|s| s.bg(theme.accent))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.session.update(cx, |s, cx| s.set_tab(Tab::Settings, cx));
                }))
                .into_any_element()
        } else {
            status_inner.into_any_element()
        });
        sidebar = sidebar.child(status);

        // Error banner over the content: dismissible, warning styled -
        // never inside the status footer, which reports connection only.
        let error_banner = error.map(|e| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_4()
                .py_2()
                .bg(theme.secondary)
                .border_b_1()
                .border_color(theme.border)
                .child(
                    Icon::new(IconName::TriangleAlert)
                        .with_size(px(15.))
                        .text_color(rgb(0xeab308)),
                )
                .child(
                    Label::new(e)
                        .text_sm()
                        .truncate()
                        .text_color(theme.muted_foreground),
                )
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
            .flex()
            .flex_row()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(sidebar)
            // Content: the active tab. Offline shows the join flow (the
            // only screen that matters then), unless the user opened
            // Settings.
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(error_banner.unwrap_or_else(|| div().into_any_element()))
                    .child(
                        div().flex_1().min_h_0().child(if connected {
                            self.render_active(cx)
                        } else if active == Tab::Settings {
                            self.settings.clone().into_any_element()
                        } else {
                            self.join.clone().into_any_element()
                        }),
                    ),
            )
    }
}
