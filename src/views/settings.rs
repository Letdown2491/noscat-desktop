//! Settings tab: status, device name, diagnostics, disconnect.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::Disableable as _;
use gpui_kit::component::label::Label;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::gpui::{
    div, AnyElement, Context, Entity, IntoElement, ParentElement as _, Styled as _,
};

use crate::app::NoscatApp;
use crate::paths;
use crate::state::{RunMode, Session};

pub fn view(session: &Entity<Session>, cx: &mut Context<NoscatApp>) -> AnyElement {
    let session = session.read(cx);
    let theme = cx.theme().clone();
    let mode = match session.mode {
        RunMode::Stopped => "stopped",
        RunMode::Node => "node (chat, files, presence)",
        RunMode::Expose => "expose (serving a local service)",
    };
    let npub = session.my_npub.clone().unwrap_or_else(|| "-".to_string());
    let confirm = session.confirm_disconnect;

    let mut status = div()
        .flex()
        .flex_col()
        .gap_1()
        .child(Label::new("Status"))
        .child(
            Label::new(format!("mode: {mode}"))
                .text_sm()
                .text_color(theme.muted_foreground),
        )
        .child(
            Label::new(format!("npub: {npub}"))
                .text_sm()
                .text_color(theme.muted_foreground),
        )
        .child(
            Label::new(format!("device name: {}", session.settings.device_name))
                .text_sm()
                .text_color(theme.muted_foreground),
        )
        .child(
            Label::new(format!("mesh dir: {}", paths::mesh_dir().display()))
                .text_sm()
                .text_color(theme.muted_foreground),
        )
        .child(
            Label::new(format!("inbox: {}", paths::inbox_dir().display()))
                .text_sm()
                .text_color(theme.muted_foreground),
        );
    if let Some(error) = &session.last_error {
        status = status.child(
            Label::new(format!("last error: {error}"))
                .text_sm()
                .text_color(theme.muted_foreground),
        );
    }

    div()
        .flex()
        .flex_col()
        .size_full()
        .gap_2()
        .px_4()
        .py_3()
        .child(status)
        .child(
            div().pt_2().child(
                // Two-step confirm: the first click arms the button, the
                // second disconnects. Replaces Android's confirm dialog.
                Button::new("disconnect")
                    .label(if confirm {
                        "Confirm disconnect?"
                    } else {
                        "Disconnect"
                    })
                    .danger()
                    .disabled(!confirm && session.mode == RunMode::Stopped)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |s, cx| {
                            if s.confirm_disconnect {
                                s.disconnect(cx);
                            } else {
                                s.confirm_disconnect = true;
                                cx.notify();
                            }
                        });
                    })),
            ),
        )
        .into_any_element()
}
