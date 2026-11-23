//! Shares tab: the desktop extras. Expose a local TCP service to peers
//! (the `noscat expose` shape) and see inbound activity.

use gpui_kit::component::label::Label;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::gpui::{
    div, AnyElement, Context, Entity, IntoElement, ParentElement as _, Styled as _,
};

use crate::app::NoscatApp;
use crate::state::Session;

pub fn view(session: &Entity<Session>, cx: &Context<NoscatApp>) -> AnyElement {
    let theme = cx.theme().clone();
    let expose_target = session.read(cx).expose_target.clone();

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .px_4()
                .py_3()
                .border_b_1()
                .border_color(theme.border)
                .child(Label::new("Expose a local service"))
                .child(
                    Label::new(match &expose_target {
                        Some(t) => format!("Peers can reach {t} by dialing your npub"),
                        None => "Not exposing anything yet.".to_string(),
                    })
                    .text_sm()
                    .text_color(theme.muted_foreground),
                ),
        )
        .child(
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(Label::new(
                    "Port forwards appear here too once added from a device row.",
                )),
        )
        .into_any_element()
}
