//! Devices tab: the mesh roster with presence dots and per-row actions
//! (chat, send file, forward).

use gpui_kit::component::label::Label;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::gpui::{
    div, px, rgb, AnyElement, Context, Entity, InteractiveElement as _, StatefulInteractiveElement as _, IntoElement, ParentElement as _,
    Styled as _,
};

use crate::app::NoscatApp;
use crate::state::Session;

pub fn view(session: &Entity<Session>, cx: &Context<NoscatApp>) -> AnyElement {
    let session = session.read(cx);
    let theme = cx.theme().clone();

    let mut list = div().flex().flex_col().size_full().id("devices-list").overflow_y_scroll();
    if session.roster.is_empty() {
        list = list.child(
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(Label::new(
                    "No devices yet. Join a mesh, or wait for the next sync.",
                )),
        );
    } else {
        list = list.children(session.roster.iter().map(|device| {
            let name = device.display_name();
            let npub_short: String = device.npub.chars().take(20).collect();
            let online = device.online();
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .px_4()
                .py_2()
                .border_b_1()
                .border_color(theme.border)
                .child(div().rounded_full().size(px(8.)).bg(if online {
                    rgb(0x22c55e)
                } else {
                    rgb(0x94a3b8)
                }))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(Label::new(name))
                        .child(
                            Label::new(npub_short)
                                .text_sm()
                                .text_color(theme.muted_foreground),
                        ),
                )
        }));
    }
    list.into_any_element()
}
