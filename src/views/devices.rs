//! Devices tab: the mesh roster with presence dots and per-row actions
//! (chat, send file, forward a local port, share the clipboard), matching
//! the Android device list. This computer's own row carries the actions
//! about itself: invite a device, rename, expose a service. Join requests
//! sit on top. Files dropped on a row are sent to that device.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Icon, Sizable as _, WindowExt as _};
use gpui_kit::gpui::prelude::FluentBuilder as _;
use gpui_kit::gpui::{
    div, img, px, AnyElement, AppContext as _, ClipboardItem, Context, Entity, ExternalPaths,
    InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window,
};
use gpui_kit_assets::IconName;

use crate::app::Tab;
use crate::palette;
use crate::state::{Presence, RunMode, Session};

/// How much of the clipboard the confirmation previews.
const CLIP_PREVIEW_CHARS: usize = 80;

/// What a member must do when its dot is amber.
const UNREACHABLE_HELP: &str = "Online but unreachable. Fix: Settings > Reachable from the internet";

/// The device avatar: its initial on its own color, with the presence dot
/// pinned to the edge.
pub fn avatar(id: &str, name: &str, dot: Presence, ring: gpui_kit::gpui::Hsla) -> impl IntoElement {
    let initial = name
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string());
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
                .bg(palette::avatar(id))
                .child(Label::new(initial).text_color(palette::on_color())),
        )
        .child(
            div()
                .absolute()
                .bottom_0()
                .right_0()
                .rounded_full()
                .size(px(10.))
                .border_2()
                .border_color(ring)
                .bg(dot.color()),
        )
}

/// Invite a device: this computer's pairing code as a QR in a dialog,
/// with a copy action. In a private mesh the code names its relays.
pub fn open_pairing_dialog(session: &Entity<Session>, window: &mut Window, cx: &mut gpui_kit::gpui::App) {
    let session = session.read(cx);
    let Some(code) = session.pairing_uri.clone().or(session.my_npub.clone()) else { return };
    let Some(qr) = crate::views::join::qr_image(&code) else { return };
    window.open_dialog(cx, move |dialog, _, cx| {
        let muted = cx.theme().muted_foreground;
        let code = code.clone();
        dialog.title("Invite a device").width(px(380.)).child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_3()
                .child(
                    Label::new(
                        "On the new device, scan this code or paste it into its join screen, \
                         then approve its request here.",
                    )
                    .text_sm()
                    .text_color(muted),
                )
                .child(div().rounded(px(8.)).overflow_hidden().child(img(qr.clone()).size(px(220.))))
                .child(
                    Button::new("copy-pairing")
                        .icon(IconName::Copy)
                        .label("Copy pairing code")
                        .on_click(move |_, window, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(code.clone()));
                            window.push_notification(Notification::success("Pairing code copied"), cx);
                        }),
                ),
        )
    });
}

pub struct DevicesView {
    session: Entity<Session>,
    /// The device whose forward editor is open, if any. Forwarding needs a
    /// local port, so the row expands an inline editor, like the Android
    /// dialog.
    forward_peer: Option<String>,
    forward_port: Option<Entity<InputState>>,
    /// Inline validation error for the open forward editor.
    forward_error: Option<String>,
    /// The clipboard waiting for its send confirmation: (device, text).
    /// Clipboards often hold passwords, so the text is previewed first.
    clip: Option<(String, String)>,
}

impl DevicesView {
    pub fn new(session: Entity<Session>, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            session,
            forward_peer: None,
            forward_port: None,
            forward_error: None,
            clip: None,
        }
    }

    /// Close the inline editors (Escape, or another action on a row).
    pub fn close_editors(&mut self, cx: &mut Context<Self>) {
        self.forward_peer = None;
        self.forward_port = None;
        self.forward_error = None;
        self.clip = None;
        cx.notify();
    }

    fn open_chat(&mut self, npub: &str, cx: &mut Context<Self>) {
        self.close_editors(cx);
        let npub = npub.to_string();
        self.session.update(cx, |s, cx| {
            s.active_chat = Some(npub.clone());
            s.open_conversation(&npub, cx);
            s.set_tab(Tab::Chat, cx);
        });
    }

    /// Read the clipboard and ask before sending it.
    fn preview_clipboard(&mut self, npub: &str, cx: &mut Context<Self>) {
        let text = cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default();
        self.close_editors(cx);
        if text.is_empty() {
            self.session.update(cx, |s, cx| {
                s.last_error = Some("The clipboard is empty.".into());
                cx.notify();
            });
            return;
        }
        self.clip = Some((npub.to_string(), text));
        cx.notify();
    }

    fn open_forward_editor(&mut self, npub: String, window: &mut Window, cx: &mut Context<Self>) {
        self.close_editors(cx);
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("e.g. 8080"));
        // Enter starts the forward.
        cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.start_forward(cx);
            }
        })
        .detach();
        input.update(cx, |i, cx| i.focus(window, cx));
        self.forward_peer = Some(npub);
        self.forward_port = Some(input);
        cx.notify();
    }

    fn start_forward(&mut self, cx: &mut Context<Self>) {
        let (Some(npub), Some(port)) = (self.forward_peer.clone(), self.forward_port.clone()) else {
            return;
        };
        match port.read(cx).value().trim().parse::<u16>() {
            Ok(port) if port > 0 => {
                self.close_editors(cx);
                self.session.update(cx, |s, cx| s.add_forward(&npub, port, cx));
            }
            _ => {
                self.forward_error = Some("Enter a port number, e.g. 8080".into());
                cx.notify();
            }
        }
    }

    /// Open the portal file picker in the background and start the transfer.
    fn pick_and_send_file(&mut self, npub: &str, cx: &mut Context<Self>) {
        let npub = npub.to_string();
        cx.spawn(async move |this, cx| {
            let picked = cx
                .background_executor()
                .spawn(async move { rfd::FileDialog::new().pick_file() })
                .await;
            if let Some(path) = picked {
                let _ = this.update(cx, |this, cx| {
                    this.session.update(cx, |s, cx| s.send_file(&npub, path, cx));
                });
            }
        })
        .detach();
    }

    /// A join request: the asker's claimed name and short ID to compare
    /// with its screen, Decline and Approve. Accented so it is not missed.
    fn request_row(&self, r: &crate::state::JoinRequest, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let who = r.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "A device".into());
        let short = crate::state::short_id(&r.npub);
        let approve_npub = r.npub.clone();
        let decline_npub = r.npub.clone();
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .px_4()
            .py_3()
            .relative()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.secondary)
            .child(div().absolute().left_0().top_0().bottom_0().w(px(4.)).bg(palette::blue()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .flex_shrink_0()
                    .size(px(36.))
                    .rounded_full()
                    .bg(palette::blue())
                    .child(Icon::new(IconName::UserPlus).with_size(px(18.)).text_color(palette::on_color())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(Label::new(format!("{who} wants to join the mesh")))
                    .child(
                        Label::new(format!(
                            "Device ID {short}. Approve only a device you are setting up: \
                             check this ID on its screen."
                        ))
                        .text_sm()
                        .text_color(theme.muted_foreground),
                    ),
            )
            .child(
                Button::new(SharedString::from(format!("decline-{}", r.npub)))
                    .label("Decline")
                    .compact()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.session.update(cx, |s, cx| s.decline_join(&decline_npub, cx));
                    })),
            )
            .child(
                Button::new(SharedString::from(format!("approve-{}", r.npub)))
                    .label(if r.approving { "Inviting..." } else { "Approve" })
                    .primary()
                    .compact()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let npub = approve_npub.clone();
                        this.session.update(cx, |s, cx| s.approve_join(npub, cx));
                    })),
            )
            .into_any_element()
    }

    fn show_pairing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        open_pairing_dialog(&self.session, window, cx);
    }

    /// This computer's actions: things about itself, not another device.
    fn self_actions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .ml_auto()
            .flex_shrink_0()
            .child(
                Button::new("self-rename")
                    .icon(IconName::Pencil)
                    .ghost()
                    .tooltip("Rename this computer")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |s, cx| {
                            s.focus_name = true;
                            s.set_tab(Tab::Settings, cx);
                        });
                    })),
            )
            .child(
                Button::new("self-expose")
                    .icon(IconName::Server)
                    .ghost()
                    .tooltip("Expose a service on this computer")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |s, cx| s.set_tab(Tab::Services, cx));
                    })),
            )
    }

    /// Another device's actions, always visible (the same ones are on
    /// right-click).
    fn peer_actions(&self, npub: &str, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_row()
            .gap_1()
            .ml_auto()
            .flex_shrink_0()
            .child(
                Button::new(SharedString::from(format!("chat-{npub}")))
                    .icon(IconName::MessageSquare)
                    .ghost()
                    .tooltip("Chat")
                    .on_click(cx.listener({
                        let npub = npub.to_string();
                        move |this, _, _, cx| this.open_chat(&npub, cx)
                    })),
            )
            .child(
                Button::new(SharedString::from(format!("file-{npub}")))
                    .icon(IconName::Paperclip)
                    .ghost()
                    .tooltip("Send a file (or drop files on the row)")
                    .on_click(cx.listener({
                        let npub = npub.to_string();
                        move |this, _, _, cx| this.pick_and_send_file(&npub, cx)
                    })),
            )
            .child(
                Button::new(SharedString::from(format!("forward-{npub}")))
                    .icon(IconName::ArrowLeftRight)
                    .ghost()
                    .tooltip("Forward a local port to its shared service")
                    .on_click(cx.listener({
                        let npub = npub.to_string();
                        move |this, _, window, cx| this.open_forward_editor(npub.clone(), window, cx)
                    })),
            )
            .child(
                Button::new(SharedString::from(format!("clipboard-{npub}")))
                    .icon(IconName::ClipboardCopy)
                    .ghost()
                    .tooltip("Share my clipboard")
                    .on_click(cx.listener({
                        let npub = npub.to_string();
                        move |this, _, _, cx| this.preview_clipboard(&npub, cx)
                    })),
            )
    }

    /// The inline forward editor under a row.
    fn forward_editor(&self, name: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let port = self.forward_port.clone().expect("editor has an input");
        div()
            .flex()
            .flex_col()
            .gap_1()
            .pt_2()
            .pl(px(48.))
            .child(
                Label::new(format!(
                    "Choose a port on this computer; connecting to it reaches the service {name} shares."
                ))
                .text_sm()
                .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(div().w_24().child(Input::new(&port)))
                    .child(
                        Button::new("start-forward")
                            .label("Start forward")
                            .compact()
                            .primary()
                            .on_click(cx.listener(|this, _, _, cx| this.start_forward(cx))),
                    )
                    .child(
                        Button::new("cancel-forward")
                            .label("Cancel")
                            .compact()
                            .on_click(cx.listener(|this, _, _, cx| this.close_editors(cx))),
                    ),
            )
            .children(self.forward_error.clone().map(|err| {
                Label::new(err).text_sm().text_color(palette::danger())
            }))
    }

    /// The clipboard confirmation under a row: a preview, Send, Cancel.
    fn clip_confirm(&self, npub: &str, name: &str, text: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let chars = text.chars().count();
        let mut preview: String = text.chars().take(CLIP_PREVIEW_CHARS).collect();
        preview = preview.replace(['\n', '\r', '\t'], " ");
        if chars > CLIP_PREVIEW_CHARS {
            preview.push_str("...");
        }
        let plural = if chars == 1 { "" } else { "s" };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .pt_2()
            .pl(px(48.))
            .child(
                Label::new(format!("Send your clipboard to {name}? It becomes {name}'s clipboard."))
                    .text_sm()
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .px_2()
                    .py_1()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .child(Label::new(format!("\"{preview}\" ({chars} character{plural})")).text_sm().truncate()),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(
                        Button::new("clip-send")
                            .label("Send")
                            .primary()
                            .compact()
                            .on_click(cx.listener({
                                let npub = npub.to_string();
                                let text = text.to_string();
                                move |this, _, _, cx| {
                                    this.clip = None;
                                    this.session.update(cx, |s, cx| s.send_clipboard(&npub, &text, cx));
                                    cx.notify();
                                }
                            })),
                    )
                    .child(
                        Button::new("clip-cancel")
                            .label("Cancel")
                            .compact()
                            .on_click(cx.listener(|this, _, _, cx| this.close_editors(cx))),
                    ),
            )
    }
}

impl Render for DevicesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let theme = cx.theme().clone();
        let requests = session.join_requests.clone();
        let roster = session.roster.clone();
        let my_npub = session.my_npub.clone();
        let up = session.mode != RunMode::Stopped;
        let mut presences: std::collections::HashMap<String, (Presence, String)> =
            roster.iter().map(|d| (d.npub.clone(), session.presence_of(d))).collect();
        // Send progress per device, built while the session is borrowed.
        let mut statuses: std::collections::HashMap<String, AnyElement> = roster
            .iter()
            .filter_map(|d| {
                crate::views::send_status::send_status(session, &d.npub, theme.muted_foreground)
                    .map(|s| (d.npub.clone(), s))
            })
            .collect();

        let mut list = div()
            .id("devices-list")
            .flex()
            .flex_col()
            .size_full()
            .overflow_y_scroll();

        // Join requests on top: a device that scanned or copied this
        // device's ID asks to be invited.
        for r in &requests {
            list = list.child(self.request_row(r, cx));
        }

        let roster_empty = roster.is_empty();
        if roster.is_empty() {
            list = list.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Label::new("No devices yet. Devices appear after the next mesh sync.")),
            );
        }

        // Two labeled sections: this computer first (noscat's members()
        // order puts it there), then everything else. The label says which
        // device is "you" without a chip on the row.
        let section = |title: &'static str| {
            div()
                .px_4()
                .pt_4()
                .pb_1()
                .child(crate::ui::section_label(title, theme.muted_foreground))
        };
        let has_others = roster.iter().any(|d| Some(&d.npub) != my_npub.as_ref());
        let mut others_labeled = false;
        let me = cx.entity().downgrade();
        for device in roster {
            if Some(&device.npub) == my_npub.as_ref() {
                list = list.child(section("THIS COMPUTER"));
            } else if !others_labeled {
                others_labeled = true;
                list = list.child(section("OTHER DEVICES"));
            }
            let npub = device.npub.clone();
            let name = device.display_name();
            let is_self = Some(&npub) == my_npub.as_ref();
            let forward_open = self.forward_peer.as_deref() == Some(npub.as_str());
            let clip = self.clip.clone().filter(|(peer, _)| *peer == npub).map(|(_, text)| text);
            // Green: data reaches it now. Amber: its advert arrives but
            // nothing connects. Grey: offline. This device is green while
            // it runs.
            let (dot, presence) = if is_self {
                (
                    if up { Presence::Connected } else { Presence::Offline },
                    if up { "this device".to_string() } else { "this device, not connected".to_string() },
                )
            } else {
                presences.remove(&npub).unwrap_or((Presence::Offline, String::new()))
            };

            // Presence line; amber explains itself on hover.
            let presence_line = div()
                .id(SharedString::from(format!("presence-{npub}")))
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .text_color(theme.muted_foreground)
                .child(Label::new(presence).text_sm())
                .when(dot == Presence::Unreachable && !is_self, |s| {
                    s.child(Icon::new(IconName::Info).with_size(px(13.)))
                        .tooltip(|window, cx| Tooltip::new(UNREACHABLE_HELP).build(window, cx))
                });

            let headline = div()
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .child(avatar(&npub, &name, dot, theme.background))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(crate::ui::name(name.clone()).truncate())
                        .child(presence_line),
                );
            let right = if is_self {
                self.self_actions(cx).into_any_element()
            } else {
                self.peer_actions(&npub, cx).into_any_element()
            };

            let mut row = div()
                .id(SharedString::from(format!("device-row-{npub}")))
                .flex()
                .flex_col()
                .px_4()
                .py_3()
                .border_b_1()
                .border_color(theme.border)
                .hover(|s| s.bg(theme.secondary))
                .child(headline.child(right));
            // Sends to this device: aligned under its name.
            if let Some(status) = statuses.remove(&npub) {
                row = row.child(div().pl(px(48.)).pt_2().max_w(px(420.)).child(status));
            }
            if forward_open {
                row = row.child(self.forward_editor(&name, cx));
            }
            if let Some(text) = clip {
                row = row.child(self.clip_confirm(&npub, &name, &text, cx));
            }

            if is_self {
                list = list.child(row);
                continue;
            }

            // Drop files on another device's row to send them there; the
            // same actions as the buttons on right-click.
            let accent = theme.accent;
            let row = row
                .drag_over::<ExternalPaths>(move |s, _, _, _| s.bg(accent))
                .on_drop(cx.listener({
                    let npub = npub.clone();
                    move |this, paths: &ExternalPaths, _, cx| {
                        for path in paths.paths().iter().filter(|p| p.is_file()) {
                            let path = path.clone();
                            this.session.update(cx, |s, cx| s.send_file(&npub, path, cx));
                        }
                    }
                }))
                .context_menu({
                    let me = me.clone();
                    let npub = npub.clone();
                    move |menu, _, _| {
                        let item = |label: &'static str, run: fn(&mut DevicesView, &str, &mut Window, &mut Context<DevicesView>)| {
                            let me = me.clone();
                            let npub = npub.clone();
                            PopupMenuItem::new(label).on_click(move |_, window, cx| {
                                let _ = me.update(cx, |this, cx| run(this, &npub, window, cx));
                            })
                        };
                        menu.item(item("Chat", |this, npub, _, cx| this.open_chat(npub, cx)))
                            .item(item("Send a file...", |this, npub, _, cx| this.pick_and_send_file(npub, cx)))
                            .item(item("Forward a local port...", |this, npub, window, cx| {
                                this.open_forward_editor(npub.to_string(), window, cx)
                            }))
                            .item(item("Share my clipboard...", |this, npub, _, cx| this.preview_clipboard(npub, cx)))
                    }
                });
            list = list.child(row);
        }

        if !roster_empty && !has_others {
            list = list.child(section("OTHER DEVICES")).child(
                crate::ui::empty_state(
                    IconName::UserPlus,
                    "No other devices yet",
                    "Show a device this computer's pairing code to add it.",
                    theme.muted_foreground,
                )
                .child(
                    Button::new("empty-invite")
                        .icon(IconName::QrCode)
                        .label("Invite a device")
                        .compact()
                        .on_click(cx.listener(|this, _, window, cx| this.show_pairing(window, cx))),
                ),
            );
        }

        list.into_any_element()
    }
}
