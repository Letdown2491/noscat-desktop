//! The root view: connection chip (shown on every tab's header), the tab
//! bar, and the active tab's view. Mirrors the Android app's four-tab
//! layout, plus the desktop Shares tab for expose/serve.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::label::Label;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::gpui::{
    div, px, rgb, AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window,
};

use crate::state::{RunMode, Session};
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let connected = session.mode != RunMode::Stopped;
        let npub = session.my_npub.clone();
        let error = session.last_error.clone();
        let theme = cx.theme().clone();
        let active = session.tab;
        let unread: usize = session.unread.values().sum();

        // Connection chip: status dot, npub, and the last error if any.
        // Clicking it opens Settings, where disconnect lives (like the
        // Android chip).
        let mut chip = div()
            .id("connection-chip")
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                this.session.update(cx, |s, cx| s.set_tab(Tab::Settings, cx));
            }))
            .child(div().rounded_full().size(px(8.)).bg(if connected {
                rgb(0x22c55e)
            } else {
                rgb(0xef4444)
            }))
            .child(Label::new(match (&npub, connected) {
                (Some(n), true) => format!("connected - {}", &n[..16.min(n.len())]),
                _ => "offline".to_string(),
            }));
        if let Some(e) = &error {
            chip =
                chip.child(Label::new(e.clone()).text_sm().text_color(theme.muted_foreground));
        }

        // Tab bar with the active tab highlighted. Chat carries the unread
        // count like the Android badge.
        let tabs = [
            (Tab::Devices, "Devices", "tab-devices"),
            (Tab::Chat, "Chat", "tab-chat"),
            (Tab::Files, "Files", "tab-files"),
            (Tab::Shares, "Shares", "tab-shares"),
            (Tab::Settings, "Settings", "tab-settings"),
        ]
        .map(|(tab, label, id)| {
            let label = if tab == Tab::Chat && unread > 0 {
                format!("{label} ({unread})")
            } else {
                label.to_string()
            };
            let button = Button::new(id).label(label).compact();
            let button = if active == tab { button.primary() } else { button };
            button.on_click(cx.listener(move |this, _, _, cx| {
                this.session.update(cx, |s, cx| s.set_tab(tab, cx));
            }))
        });

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            // Header: connection chip plus tab bar, on every tab like the
            // Android app.
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(chip)
                    .child(div().flex().flex_row().gap_1().children(tabs)),
            )
            // Body: the active tab. Offline shows the join flow instead.
            .child(
                div().flex_1().min_h_0().child(if connected {
                    self.render_active(cx)
                } else {
                    self.join.clone().into_any_element()
                }),
            )
    }
}
