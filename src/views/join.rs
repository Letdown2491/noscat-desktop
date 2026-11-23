//! Join flow: the desktop counterpart of the Android app's QR join. The
//! desktop has no camera, so the flow is paste and display: paste the
//! owner's npub (the one whose QR was scanned on the phone) to address the
//! invite request, and show this device's own npub and QR for the owner to
//! invite it back.

use gpui_kit::component::button::Button;
use gpui_kit::component::label::Label;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::gpui::{
    div, px, AnyElement, Context, Entity, IntoElement as _, ParentElement as _, Styled as _,
};

use crate::app::NoscatApp;
use crate::state::Session;

pub fn view(session: &Entity<Session>, cx: &mut Context<NoscatApp>) -> AnyElement {
    let session = session.read(cx);
    let theme = cx.theme().clone();
    let npub = session.my_npub.clone().unwrap_or_else(|| "-".to_string());

    div()
        .flex()
        .flex_col()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .child(Label::new("Connect to a mesh").text_size(px(18.)))
        .child(
            Label::new(
                "Paste the owner's npub to request an invite, or show your \
                 npub below to the owner (the phone can scan it).",
            )
            .text_sm()
            .text_color(theme.muted_foreground),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .gap_2()
                .child(Button::new("request-join").label("Request invite"))
                .child(Button::new("wait-join").label("I was invited, join")),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .p_4()
                .border_1()
                .border_color(theme.border)
                .rounded(px(8.))
                .child(
                    Label::new("This device's npub:")
                        .text_sm()
                        .text_color(theme.muted_foreground),
                )
                .child(Label::new(npub)),
        )
        .into_any_element()
}
