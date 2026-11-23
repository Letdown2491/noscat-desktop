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
use crate::views::join::JoinView;
use crate::views::{chat, devices, files, settings, shares};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Devices,
    Chat,
    Files,
    Shares,
    Settings,
}

const TABS: [(Tab, &str, &str); 5] = [
    (Tab::Devices, "Devices", "tab-devices"),
    (Tab::Chat, "Chat", "tab-chat"),
    (Tab::Files, "Files", "tab-files"),
    (Tab::Shares, "Shares", "tab-shares"),
    (Tab::Settings, "Settings", "tab-settings"),
];

pub struct NoscatApp {
    pub(crate) session: Entity<Session>,
    join: Entity<JoinView>,
    active: Tab,
}

impl NoscatApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let session = cx.new(|cx| Session::new(cx));
        // Re-render whenever session state changes.
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        let join = cx.new(|cx| JoinView::new(session.clone(), window, cx));
        Self {
            session,
            join,
            active: Tab::Devices,
        }
    }

    fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.active = tab;
        // Leaving the Settings tab cancels a pending disconnect confirm.
        if tab != Tab::Settings {
            self.session.update(cx, |s, cx| {
                s.confirm_disconnect = false;
                cx.notify();
            });
        }
        cx.notify();
    }

    fn render_active(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.active {
            Tab::Devices => devices::view(&self.session, cx),
            Tab::Chat => chat::view(&self.session, cx),
            Tab::Files => files::view(&self.session, cx),
            Tab::Shares => shares::view(&self.session, cx),
            Tab::Settings => settings::view(&self.session, cx),
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
        let active = self.active;

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
            .on_click(cx.listener(|this, _, _, cx| this.set_tab(Tab::Settings, cx)))
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

        // Tab bar with the active tab highlighted.
        let tabs = TABS.map(|(tab, label, id)| {
            let button = Button::new(id).label(label).compact();
            let button = if active == tab { button.primary() } else { button };
            button.on_click(cx.listener(move |this, _, _, cx| {
                this.set_tab(tab, cx);
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
