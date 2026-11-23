//! Files tab: the inbox and live transfer progress, both directions, like
//! the Android Files tab.

use gpui_kit::component::button::Button;
use gpui_kit::component::label::Label;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::gpui::{
    div, rgb, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window,
};

use crate::paths;
use crate::state::{Session, TransferState};

pub struct FilesView {
    session: Entity<Session>,
}

impl FilesView {
    pub fn new(session: Entity<Session>, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self { session }
    }
}

impl Render for FilesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let theme = cx.theme().clone();
        let inbox = paths::inbox_dir();

        let mut list = div()
            .id("files-list")
            .flex()
            .flex_col()
            .size_full()
            .overflow_y_scroll()
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
                    .child(
                        Label::new(format!("Inbox: {}", inbox.display()))
                            .text_sm()
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        Button::new("open-inbox")
                            .label("Open folder")
                            .compact()
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.spawn(async move |_, cx| {
                                    let inbox = paths::inbox_dir();
                                    cx.background_executor()
                                        .spawn(async move {
                                            let _ = std::process::Command::new("xdg-open")
                                                .arg(inbox)
                                                .spawn();
                                        })
                                        .await;
                                })
                                .detach();
                            })),
                    ),
            );

        if session.transfers.is_empty() {
            list = list.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Label::new(
                        "No transfers. Inbound files land in the inbox automatically.",
                    )),
            );
        }

        for t in session.transfers.clone() {
            let name = t
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "file".into());
            let pct = if t.total > 0 { t.sent * 100 / t.total } else { 0 };
            let state = match t.state {
                TransferState::Active => format!("{pct}%"),
                TransferState::Done => "done".to_string(),
                TransferState::Failed => "failed".to_string(),
            };
            let direction = if t.outbound { "to" } else { "from" };
            list = list.child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(Label::new(format!("{direction} {} {name}", t.peer)))
                    .child(
                        Label::new(state).text_sm().text_color(match t.state {
                            TransferState::Failed => gpui_kit::gpui::Hsla::from(rgb(0xf87171)),
                            TransferState::Done => gpui_kit::gpui::Hsla::from(rgb(0x22c55e)),
                            TransferState::Active => theme.muted_foreground,
                        }),
                    ),
            );
        }

        list.into_any_element()
    }
}
