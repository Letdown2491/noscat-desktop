//! Files tab: inbox plus live transfer progress.

use gpui_kit::component::label::Label;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::gpui::{
    div, AnyElement, Context, Entity, InteractiveElement as _, StatefulInteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
};

use crate::app::NoscatApp;
use crate::paths;
use crate::state::{Session, TransferState};

pub fn view(session: &Entity<Session>, cx: &Context<NoscatApp>) -> AnyElement {
    let session = session.read(cx);
    let theme = cx.theme().clone();
    let inbox = paths::inbox_dir();

    let mut list = div().flex().flex_col().size_full().id("files-list").overflow_y_scroll().child(
        div().px_4().py_2().child(
            Label::new(format!("Inbox: {}", inbox.display()))
                .text_sm()
                .text_color(theme.muted_foreground),
        ),
    );
    if session.transfers.is_empty() {
        list = list.child(
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(Label::new("No transfers yet.")),
        );
    } else {
        list = list.children(session.transfers.iter().map(|t| {
            let name = t
                .path
                .as_ref()
                .map(|p| {
                    p.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default()
                })
                .unwrap_or_default();
            let pct = if t.total > 0 { t.sent * 100 / t.total } else { 0 };
            let state = match t.state {
                TransferState::Active => format!("{pct}%"),
                TransferState::Done => "done".to_string(),
                TransferState::Failed => "failed".to_string(),
            };
            let direction = if t.outbound { "to" } else { "from" };
            div()
                .flex()
                .flex_row()
                .justify_between()
                .px_4()
                .py_2()
                .border_b_1()
                .border_color(theme.border)
                .child(Label::new(format!("{} {} {}", direction, t.peer, name)))
                .child(
                    Label::new(state)
                        .text_sm()
                        .text_color(theme.muted_foreground),
                )
        }));
    }
    list.into_any_element()
}
