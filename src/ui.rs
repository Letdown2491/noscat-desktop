//! Shared building blocks, so every page speaks the same visual language:
//! the type scale (page title 18 semibold, section label 12 semibold muted,
//! body 14 with names medium, details 12), empty states, and the logo.

use std::sync::{Arc, LazyLock};

use gpui_kit::component::label::Label;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::gpui::{
    div, img, px, Div, FontWeight, Hsla, Image, ImageFormat, IntoElement, ParentElement as _,
    SharedString, Styled as _,
};
use gpui_kit_assets::IconName;

/// The noscat mark (the cat in its mesh), rendered from the source SVG.
static LOGO: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(ImageFormat::Svg, include_bytes!("../assets/noscat.svg").to_vec()))
});

pub fn logo(size: f32) -> impl IntoElement {
    img(LOGO.clone()).size(px(size)).flex_shrink_0()
}

/// A page's title.
pub fn page_title(text: impl Into<SharedString>) -> Label {
    Label::new(text).text_size(px(18.)).font_weight(FontWeight::SEMIBOLD)
}

/// A section's label: small, semibold, muted, so sections are scannable
/// without competing with the content.
pub fn section_label(text: impl Into<SharedString>, muted: Hsla) -> Label {
    Label::new(text)
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(muted)
}

/// A name in a list (device, conversation, file): medium weight.
pub fn name(text: impl Into<SharedString>) -> Label {
    Label::new(text).font_weight(FontWeight::MEDIUM)
}

/// The empty state every list uses: icon, title, one line, and optionally
/// the action that fills it.
pub fn empty_state(
    icon: IconName,
    title: impl Into<SharedString>,
    hint: impl Into<SharedString>,
    muted: Hsla,
) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .py_16()
        .px_4()
        .child(Icon::new(icon).with_size(px(28.)).text_color(muted))
        .child(Label::new(title).font_weight(FontWeight::MEDIUM))
        .child(Label::new(hint).text_sm().text_color(muted))
}
