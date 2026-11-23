//! Chat tab: conversation list and bubbles with optimistic sending.

use gpui_kit::component::label::Label;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::gpui::{
    div, AnyElement, Context, Entity, InteractiveElement as _, StatefulInteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
};

use crate::app::NoscatApp;
use crate::state::Session;

pub fn view(session: &Entity<Session>, cx: &Context<NoscatApp>) -> AnyElement {
    let session = session.read(cx);
    let theme = cx.theme().clone();

    let mut list = div().flex().flex_col().size_full().id("chat-list").overflow_y_scroll();
    if session.conversations.is_empty() {
        list = list.child(
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(Label::new(
                    "No conversations yet. Pick a device on the Devices tab.",
                )),
        );
    } else {
        list = list.children(session.conversations.iter().map(|(peer, messages)| {
            let unread = session.unread.get(peer).copied().unwrap_or(0);
            let last = messages
                .last()
                .map(|m| m.text.clone())
                .unwrap_or_default();
            let mut row = div()
                .flex()
                .flex_col()
                .px_4()
                .py_2()
                .border_b_1()
                .border_color(theme.border)
                .child(Label::new(peer.clone()))
                .child(
                    Label::new(last).text_sm().text_color(theme.muted_foreground),
                );
            if unread > 0 {
                row = row.child(Label::new(format!("{unread} unread")).text_sm());
            }
            row
        }));
    }
    list.into_any_element()
}
