//! Chat tab: conversation list, and the open conversation with optimistic
//! bubbles and an input row - the Android chat parity.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::gpui::{
    div, px, rgb, AppContext as _, AnyElement, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_kit_assets::IconName;

use crate::state::{MessageState, Session};

/// Local clock time for a bubble timestamp, e.g. "14:05". Empty when the
/// timestamp is unusable (never expected).
fn clock_time(at_ms: u64) -> String {
    let Ok(ts) = jiff::Timestamp::from_millisecond(at_ms as i64) else {
        return String::new();
    };
    ts.to_zoned_datetime(jiff::tz::TimeZone::system())
        .strftime("%H:%M")
        .to_string()
}

pub struct ChatView {
    session: Entity<Session>,
    input: Entity<InputState>,
}

impl ChatView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Message")
        });
        // Enter sends, like every desktop chat client. The peer is read at
        // send time so the subscription works for any open conversation.
        cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                let peer = this.session.read(cx).active_chat.clone();
                if let Some(peer) = peer {
                    this.send(&peer, window, cx);
                }
            }
        })
        .detach();
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

        // Entries: every conversation, plus every *other* online device
        // without one (Android lets you start a chat from any device). Our
        // own device is in the roster but is nobody's chat partner.
        let my_npub = session.my_npub.clone();
        let mut peers: Vec<String> = session.conversations.keys().cloned().collect();
        for device in &session.roster {
            if device.online() && Some(&device.npub) != my_npub.as_ref() {
                peers.push(device.npub.clone());
            }
        }
        peers.retain(|peer| Some(peer) != my_npub.as_ref());
        peers.sort();
        peers.dedup();

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
            let device = session.roster.iter().find(|d| d.npub == peer);
            let name = device
                .map(|d| d.display_name())
                .unwrap_or_else(|| "device".to_string());
            let online = device.map(|d| d.online()).unwrap_or(false);
            let last = session
                .conversations
                .get(&peer)
                .and_then(|m| m.last())
                .map(|m| (m.text.clone(), m.at_ms));
            let unread = session.unread.get(&peer).copied().unwrap_or(0);

            // Chat list row, the messenger shape: avatar with presence dot,
            // name over the last message preview, time and unread count on
            // the right.
            let initial = name
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_else(|| "?".to_string());
            let row = div()
                .id(SharedString::from(format!("conversation-row-{peer}")))
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .px_4()
                .py_3()
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
                        .relative()
                        .flex_shrink_0()
                        .child(
                            div()
                                .rounded_full()
                                .size(px(36.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(theme.accent)
                                .child(Label::new(initial).text_color(theme.accent_foreground)),
                        )
                        .child(
                            div()
                                .absolute()
                                .bottom_0()
                                .right_0()
                                .rounded_full()
                                .size(px(10.))
                                .border_2()
                                .border_color(theme.background)
                                .bg(if online {
                                    rgb(0x22c55e)
                                } else {
                                    rgb(0x94a3b8)
                                }),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .flex_1()
                        .child(Label::new(name))
                        .child(
                            Label::new(
                                last.as_ref()
                                    .map(|(text, _)| text.clone())
                                    .unwrap_or_else(|| "no messages yet".to_string()),
                            )
                            .text_sm()
                            .truncate()
                            .text_color(theme.muted_foreground),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_end()
                        .gap_1()
                        .flex_shrink_0()
                        .children(last.as_ref().map(|(_, at_ms)| {
                            Label::new(clock_time(*at_ms))
                                .text_xs()
                                .text_color(theme.muted_foreground)
                        }))
                        .children(if unread > 0 {
                            Some(
                                div()
                                    .rounded_full()
                                    .px_1p5()
                                    .min_w(px(18.))
                                    .text_center()
                                    .bg(rgb(0x2563eb))
                                    .child(
                                        Label::new(unread.to_string())
                                            .text_xs()
                                            .text_color(rgb(0xffffff)),
                                    ),
                            )
                        } else {
                            None
                        }),
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
            .unwrap_or_else(|| "device".to_string());

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
            let time = clock_time(m.at_ms);
            let state_text = match m.state {
                MessageState::Sending => format!("{time} - sending"),
                MessageState::Sent => time,
                // The member acknowledged it: the Android app's
                // delivered tick.
                MessageState::Delivered => format!("{time} - delivered"),
                MessageState::Failed => format!("{time} - failed"),
            };
            let meta = if state_text.is_empty() {
                None
            } else {
                Some(state_text)
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
                        .child(match meta {
                            None => div().into_any_element(),
                            Some(text) => Label::new(text)
                                .text_xs()
                                .text_color(if m.state == MessageState::Failed {
                                    gpui_kit::gpui::Hsla::from(rgb(0xfca5a5))
                                } else {
                                    gpui_kit::gpui::Hsla::from(gpui_kit::gpui::rgba(0xffffff66))
                                })
                                .into_any_element(),
                        }),
                );
            bubbles = bubbles.child(
                div()
                    .flex()
                    .flex_row()
                    .px_4()
                    // Both sides fill the row so the bubble's max_w_3_4 has a
                    // width to resolve against (a content-sized wrapper
                    // shrank incoming bubbles to one word per line).
                    .child(if m.outbound {
                        div().flex_1().flex().justify_end().child(bubble)
                    } else {
                        div().flex_1().flex().justify_start().child(bubble)
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
                    .icon(IconName::Send)
                    .primary()
                    .compact()
                    .tooltip("Send (Enter)")
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
                            .icon(IconName::ArrowLeft)
                            .ghost()
                            .compact()
                            .tooltip("Back")
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
