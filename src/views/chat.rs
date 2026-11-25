//! Chat tab, the desktop messenger shape: the conversation list on the
//! left, the open conversation on the right (optimistic bubbles and an
//! input row). Files dropped on the conversation are sent to that device.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::gpui::prelude::FluentBuilder as _;
use gpui_kit::gpui::{
    div, px, AnyElement, AppContext as _, Context, Entity, ExternalPaths,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_kit_assets::IconName;

use crate::palette;
use crate::state::{MessageState, Presence, Session};
use crate::views::devices::avatar;

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

/// The list's time column: the clock today, the date before.
fn list_time(at_ms: u64) -> String {
    let Ok(ts) = jiff::Timestamp::from_millisecond(at_ms as i64) else {
        return String::new();
    };
    let tz = jiff::tz::TimeZone::system();
    let at = ts.to_zoned_datetime(tz.clone());
    if at.date() == jiff::Timestamp::now().to_zoned_datetime(tz).date() {
        at.strftime("%H:%M").to_string()
    } else {
        at.strftime("%b %-d").to_string()
    }
}

pub struct ChatView {
    session: Entity<Session>,
    input: Entity<InputState>,
    /// The open conversation's scroll position.
    scroll: ScrollHandle,
    /// The conversation and message count last rendered: a change means a
    /// new conversation (scroll to the end) or a new message (follow it,
    /// unless the user scrolled up to read).
    rendered: Option<(String, usize)>,
}

impl ChatView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Message"));
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
        Self {
            session,
            input,
            scroll: ScrollHandle::new(),
            rendered: None,
        }
    }
}

impl Render for ChatView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let active = self.session.read(cx).active_chat.clone();
        let list = self.render_conversations(active.as_deref(), cx);
        let right = match active {
            Some(peer) => self.render_conversation(&peer, cx),
            None => {
                self.rendered = None;
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(
                        Icon::new(IconName::MessageSquare)
                            .with_size(px(28.))
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        Label::new("Pick a conversation")
                            .text_sm()
                            .text_color(theme.muted_foreground),
                    )
                    .into_any_element()
            }
        };
        div()
            .flex()
            .flex_row()
            .size_full()
            .child(
                div()
                    .w(px(280.))
                    .flex_shrink_0()
                    .h_full()
                    .border_r_1()
                    .border_color(theme.border)
                    .child(list),
            )
            .child(div().flex_1().min_w_0().h_full().child(right))
    }
}

impl ChatView {
    /// The conversation list: one row per member we have (or can start) a
    /// chat with, the open one highlighted.
    fn render_conversations(&self, active: Option<&str>, cx: &mut Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let theme = cx.theme().clone();

        // Every member is listed (a message can arrive later), latest
        // conversation first, then the rest in device order. Our own
        // device is in the roster but is nobody's chat partner.
        let my_npub = session.my_npub.clone();
        let mut peers: Vec<String> = session.conversations.keys().cloned().collect();
        for device in &session.roster {
            peers.push(device.npub.clone());
        }
        peers.retain(|peer| Some(peer) != my_npub.as_ref());
        peers.sort();
        peers.dedup();
        let last_at = |peer: &String| session.conversations.get(peer).and_then(|m| m.last()).map_or(0, |m| m.at_ms);
        let rank = |peer: &String| session.roster.iter().position(|d| &d.npub == peer).unwrap_or(usize::MAX);
        peers.sort_by(|a, b| last_at(b).cmp(&last_at(a)).then_with(|| rank(a).cmp(&rank(b))).then_with(|| a.cmp(b)));

        let mut list = div()
            .id("chat-list")
            .flex()
            .flex_col()
            .size_full()
            .overflow_y_scroll();

        if peers.is_empty() {
            list = list.child(
                div()
                    .p_4()
                    .child(
                        Label::new("No other devices in the mesh yet.")
                            .text_sm()
                            .text_color(theme.muted_foreground),
                    ),
            );
        }

        let rows: Vec<_> = peers
            .into_iter()
            .map(|peer| {
                let device = session.roster.iter().find(|d| d.npub == peer);
                let name = device.map(|d| d.display_name()).unwrap_or_else(|| "device".to_string());
                let dot = device.map(|d| d.presence()).unwrap_or(Presence::Offline);
                let last = session
                    .conversations
                    .get(&peer)
                    .and_then(|m| m.last())
                    .map(|m| (m.text.clone(), m.at_ms, m.outbound));
                let unread = session.unread.get(&peer).copied().unwrap_or(0);
                (peer, name, dot, last, unread)
            })
            .collect();

        for (peer, name, dot, last, unread) in rows {
            let selected = active == Some(peer.as_str());
            let preview = match &last {
                Some((text, _, true)) => format!("You: {text}"),
                Some((text, _, false)) => text.clone(),
                None => "No messages yet".to_string(),
            };
            let row = div()
                .id(SharedString::from(format!("conversation-row-{peer}")))
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .px_3()
                .py_2p5()
                .mx_2()
                .my_0p5()
                .rounded(px(8.))
                .cursor_pointer()
                .when(selected, |s| s.bg(theme.accent))
                .when(!selected, |s| s.hover(|s| s.bg(theme.secondary)))
                .on_click(cx.listener({
                    let peer = peer.clone();
                    move |this, _, window, cx| {
                        this.session.update(cx, |s, cx| {
                            s.active_chat = Some(peer.clone());
                            s.open_conversation(&peer, cx);
                        });
                        this.input.update(cx, |i, cx| i.focus(window, cx));
                    }
                }))
                .child(avatar(&peer, &name, dot, theme.background))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .flex_1()
                        .child(Label::new(name).truncate())
                        .child(
                            Label::new(preview)
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
                        .children(last.as_ref().map(|(_, at_ms, _)| {
                            Label::new(list_time(*at_ms))
                                .text_xs()
                                .text_color(theme.muted_foreground)
                        }))
                        .children((unread > 0).then(|| {
                            div()
                                .rounded_full()
                                .px_1p5()
                                .min_w(px(18.))
                                .text_center()
                                .bg(palette::blue())
                                .child(
                                    Label::new(unread.to_string())
                                        .text_xs()
                                        .text_color(palette::on_color()),
                                )
                        })),
                );
            list = list.child(row);
        }

        list.into_any_element()
    }

    /// The open conversation: header, bubbles, input row. Dropping files
    /// anywhere on it sends them to this device.
    fn render_conversation(&mut self, peer: &str, cx: &mut Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let theme = cx.theme().clone();
        let messages = session.conversations.get(peer).cloned().unwrap_or_default();
        let device = session.roster.iter().find(|d| d.npub == peer);
        let name = device.map(|d| d.display_name()).unwrap_or_else(|| "device".to_string());
        let dot = device.map(|d| d.presence()).unwrap_or(Presence::Offline);
        let presence = device.map(|d| d.presence_text()).unwrap_or_default();

        // Follow the conversation: a newly opened one starts at its end, and
        // a new message scrolls into view when the user was already at the
        // bottom or sent it.
        let count = messages.len();
        match &self.rendered {
            Some((p, n)) if p == peer && *n == count => {}
            Some((p, _)) if p == peer => {
                let max = self.scroll.max_offset().y;
                let at_bottom = max + self.scroll.offset().y < px(64.);
                let mine = messages.last().is_some_and(|m| m.outbound);
                if at_bottom || mine {
                    self.scroll.scroll_to_bottom();
                }
            }
            _ => self.scroll.scroll_to_bottom(),
        }
        self.rendered = Some((peer.to_string(), count));

        // "delivered" on the newest delivered message only; older ones
        // are implied, as in every messenger.
        let last_delivered = messages
            .iter()
            .rposition(|m| m.outbound && m.state == MessageState::Delivered);

        let mut bubbles = div()
            .id("bubbles")
            .track_scroll(&self.scroll)
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_2()
            .py_4()
            .overflow_y_scroll();

        if messages.is_empty() {
            bubbles = bubbles.child(
                div().flex_1().flex().items_center().justify_center().child(
                    Label::new(format!("Say hi to {name}, or drop a file here to send it."))
                        .text_sm()
                        .text_color(theme.muted_foreground),
                ),
            );
        }

        for (index, m) in messages.into_iter().enumerate() {
            let time = clock_time(m.at_ms);
            let meta = match m.state {
                MessageState::Sending => format!("{time} · sending"),
                MessageState::Delivered if Some(index) == last_delivered => format!("{time} · delivered"),
                MessageState::Failed if m.outbound => format!("{time} · not delivered"),
                _ => time,
            };
            let failed = m.outbound && m.state == MessageState::Failed;
            // Sent: the app's blue with white text, readable in both themes
            // (the theme's primary is near white in dark mode). Received:
            // the theme's secondary surface.
            let (bg, fg) = if m.outbound {
                (palette::blue(), palette::on_color())
            } else {
                (theme.secondary, theme.secondary_foreground)
            };
            let meta_color = if failed { palette::danger_on_blue() } else { fg.opacity(0.7) };

            // Under the bubble: Retry (failed messages) and Delete. Delete
            // shows on hover only, except on a failed message.
            let link = |id: String, label: &'static str| {
                div()
                    .id(SharedString::from(id))
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .cursor_pointer()
                    .hover(|s| s.text_color(theme.foreground))
                    .child(label)
            };
            let group = SharedString::from(format!("message-{index}"));
            let mut actions = div().flex().flex_row().gap_3().mt_1();
            if failed {
                actions = actions.child(link(format!("retry-{index}"), "Retry").on_click(cx.listener({
                    let peer = peer.to_string();
                    move |this, _, _, cx| this.session.update(cx, |s, cx| s.retry_chat(&peer, index, cx))
                })));
            }
            let delete = link(format!("delete-{index}"), "Delete").on_click(cx.listener({
                let peer = peer.to_string();
                move |this, _, _, cx| this.session.update(cx, |s, cx| s.delete_chat(&peer, index, cx))
            }));
            actions = actions.child(if failed {
                delete
            } else {
                delete.invisible().group_hover(group.clone(), |s| s.visible())
            });
            let bubble = div()
                .max_w_3_4()
                .px_3()
                .py_2()
                .rounded(px(10.))
                .bg(bg)
                .text_color(fg)
                .when(failed, |b| b.border_1().border_color(palette::danger()))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(Label::new(m.text).text_color(fg))
                        .child(Label::new(meta).text_xs().text_color(meta_color)),
                );
            bubbles = bubbles.child(
                div()
                    .flex()
                    .flex_row()
                    .px_4()
                    // Both sides fill the row so the bubble's max_w_3_4 has a
                    // width to resolve against (a content-sized wrapper
                    // shrank incoming bubbles to one word per line).
                    .group(group)
                    .child(if m.outbound {
                        div().flex_1().flex().flex_col().items_end().child(bubble).child(actions)
                    } else {
                        div().flex_1().flex().flex_col().items_start().child(bubble).child(actions)
                    }),
            );
        }

        // An offline member cannot take a message now; say so before the
        // user finds out from a failed bubble.
        let offline_note = (dot == Presence::Offline).then(|| {
            div()
                .px_4()
                .py_1p5()
                .border_t_1()
                .border_color(theme.border)
                .child(
                    Label::new(format!("{name} is offline. Messages sent now will not be delivered."))
                        .text_xs()
                        .text_color(theme.muted_foreground),
                )
        });

        let input_row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .border_t_1()
            .border_color(theme.border)
            .child(
                Button::new("attach")
                    .icon(IconName::Paperclip)
                    .ghost()
                    .compact()
                    .tooltip("Send a file")
                    .on_click(cx.listener({
                        let peer = peer.to_string();
                        move |this, _, _, cx| this.pick_and_send_file(&peer, cx)
                    })),
            )
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

        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .child(avatar(peer, &name, dot, theme.background))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(Label::new(name).truncate())
                    .child(Label::new(presence).text_xs().text_color(theme.muted_foreground)),
            );

        div()
            .id("conversation")
            .flex()
            .flex_col()
            .size_full()
            .drag_over::<ExternalPaths>(move |s, _, _, _| s.bg(theme.accent.opacity(0.35)))
            .on_drop(cx.listener({
                let peer = peer.to_string();
                move |this, paths: &ExternalPaths, _, cx| {
                    for path in paths.paths().iter().filter(|p| p.is_file()) {
                        let path = path.clone();
                        this.session.update(cx, |s, cx| s.send_file(&peer, path, cx));
                    }
                }
            }))
            .child(header)
            .child(bubbles)
            .children(offline_note)
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

    /// Open the portal file picker in the background and start the transfer.
    fn pick_and_send_file(&mut self, peer: &str, cx: &mut Context<Self>) {
        let peer = peer.to_string();
        cx.spawn(async move |this, cx| {
            let picked = cx
                .background_executor()
                .spawn(async move { rfd::FileDialog::new().pick_file() })
                .await;
            if let Some(path) = picked {
                let _ = this.update(cx, |this, cx| {
                    this.session.update(cx, |s, cx| s.send_file(&peer, path, cx));
                });
            }
        })
        .detach();
    }
}
