//! Chat tab: conversation list, and the open conversation with optimistic
//! bubbles and an input row - the Android chat parity.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::gpui::{
    div, px, rgb, AppContext as _, AnyElement, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
};

use crate::state::{MessageState, Session};

pub struct ChatView {
    session: Entity<Session>,
    input: Entity<InputState>,
}

impl ChatView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Message")
        });
        Self { session, input }
    }
}

impl Render for ChatView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);

        match session.active_chat.clone() {
            Some(peer) => self.render_conversation(&peer, cx),
            None => self.render_conversations(cx),
        }
    }
}

impl ChatView {
    /// The conversation list: one row per peer we have (or can start) a
    /// chat with.
    fn render_conversations(&self, cx: &mut Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let theme = cx.theme().clone();

        // Entries: every conversation, plus every online device without one
        // (Android lets you start a chat from any device).
        let mut peers: Vec<String> = session.conversations.keys().cloned().collect();
        for device in &session.roster {
            if device.online() && !peers.contains(&device.npub) {
                peers.push(device.npub.clone());
            }
        }

        let mut list = div()
            .id("chat-list")
            .flex()
            .flex_col()
            .size_full()
            .overflow_y_scroll();

        if peers.is_empty() {
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
        }

        for peer in peers {
            let name = session
                .roster
                .iter()
                .find(|d| d.npub == peer)
                .map(|d| d.display_name())
                .unwrap_or_else(|| peer.clone());
            let last = session
                .conversations
                .get(&peer)
                .and_then(|m| m.last())
                .map(|m| m.text.clone())
                .unwrap_or_default();
            let unread = session.unread.get(&peer).copied().unwrap_or(0);
            let row = div()
                .id(SharedString::from(format!("conversation-row-{peer}")))
                .flex()
                .flex_col()
                .px_4()
                .py_2()
                .border_b_1()
                .border_color(theme.border)
                .hover(|s| s.bg(theme.secondary))
                .on_click(cx.listener({
                    let peer = peer.clone();
                    move |this, _, _, cx| {
                        this.session.update(cx, |s, cx| {
                            s.active_chat = Some(peer.clone());
                            s.open_conversation(&peer, cx);
                        });
                    }
                }))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_between()
                        .child(Label::new(name))
                        .child(if unread > 0 {
                            Label::new(unread.to_string())
                                .text_sm()
                                .text_color(rgb(0x3b82f6))
                                .into_any_element()
                        } else {
                            div().into_any_element()
                        }),
                )
                .child(
                    Label::new(last)
                        .text_sm()
                        .text_color(theme.muted_foreground),
                );
            list = list.child(row);
        }

        list.into_any_element()
    }

    /// The open conversation: bubbles plus the input row.
    fn render_conversation(&self, peer: &str, cx: &mut Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let theme = cx.theme().clone();
        let messages = session.conversations.get(peer).cloned().unwrap_or_default();
        let name = session
            .roster
            .iter()
            .find(|d| d.npub == peer)
            .map(|d| d.display_name())
            .unwrap_or_else(|| peer.to_string());

        let mut bubbles = div()
            .id("bubbles")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .overflow_y_scroll();

        if messages.is_empty() {
            bubbles = bubbles.child(
                div().flex_1().flex().items_center().justify_center().child(
                    Label::new(format!("Say hi to {name}"))
                        .text_sm()
                        .text_color(theme.muted_foreground),
                ),
            );
        }

        for m in messages {
            let state_text = match m.state {
                MessageState::Sending => "sending",
                MessageState::Sent => "",
                MessageState::Failed => "failed",
            };
            let bubble = div()
                .max_w_3_4()
                .px_3()
                .py_2()
                .rounded(px(10.))
                .bg(if m.outbound {
                    rgb(0x2563eb)
                } else {
                    rgb(0x3f3f46)
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(Label::new(m.text))
                        .child(if state_text.is_empty() {
                            div().into_any_element()
                        } else {
                            Label::new(state_text)
                                .text_sm()
                                .text_color(if m.state == MessageState::Failed {
                                    gpui_kit::gpui::Hsla::from(rgb(0xf87171))
                                } else {
                                    theme.muted_foreground
                                })
                                .into_any_element()
                        }),
                );
            bubbles = bubbles.child(
                div()
                    .flex()
                    .flex_row()
                    .px_4()
                    .child(if m.outbound {
                        div().flex_1().flex().justify_end().child(bubble)
                    } else {
                        div().child(bubble)
                    }),
            );
        }

        // Input row.
        let input_row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .border_t_1()
            .border_color(theme.border)
            .child(div().flex_1().child(Input::new(&self.input)))
            .child(
                Button::new("send")
                    .label("Send")
                    .primary()
                    .compact()
                    .on_click(cx.listener({
                        let peer = peer.to_string();
                        move |this, _, window, cx| this.send(&peer, window, cx)
                    })),
            );

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                // Conversation header with a back button, like Android.
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        Button::new("back")
                            .label("< Back")
                            .compact()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.session.update(cx, |s, cx| {
                                    s.active_chat = None;
                                    cx.notify();
                                });
                            })),
                    )
                    .child(Label::new(name)),
            )
            .child(bubbles)
            .child(input_row)
            .into_any_element()
    }

    fn send(&mut self, peer: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        self.session.update(cx, |s, cx| s.send_chat(peer, &text, cx));
        self.input.update(cx, |i, cx| {
            i.set_value("", window, cx);
        });
        cx.notify();
    }
}
