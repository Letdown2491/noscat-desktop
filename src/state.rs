//! The `Session` entity: the desktop counterpart of the Android app's
//! `ClientHolder`. Owns the running `noscat_client::Client`, the event pump
//! into GPUI, and everything the tabs read: roster cache, chat history,
//! unread counts, transfer state, forwards.
//!
//! Threading rules (see AGENTS.md): events arrive on a pump thread and are
//! funneled into `Session::handle_event` on the main thread; blocking client
//! calls run on the background executor and come back as state updates.
//!
//! The allow(dead_code) covers the connect/join milestone wiring: start_node
//! and the command methods light up when the join flow calls them.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use gpui_kit::gpui::{ClipboardItem, Context, WeakEntity};
use noscat_client::{Client, ClientConfig, Event, IdentitySource};

use crate::app::Tab;
use crate::mesh::MeshMember;
use crate::paths::{self, Settings};

/// One device in the roster: mesh membership enriched with advert name and
/// presence, aged forward across flaky relay fetches.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeviceEntry {
    pub npub: String,
    pub is_owner: bool,
    pub name: Option<String>,
    /// Seconds since the advert was last seen, from the freshest sync.
    pub last_seen_secs: Option<u64>,
    /// Monotonic timestamp (millis) of the last sync that saw this device,
    /// used to age presence forward between syncs.
    pub synced_at_ms: u64,
}

impl DeviceEntry {
    pub fn display_name(&self) -> String {
        match &self.name {
            Some(n) => n.clone(),
            None => {
                let tail: String = self.npub.chars().rev().take(8).collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                format!("device {tail}")
            }
        }
    }

    /// Online = the advert arrived within the advert TTL (an hour). Absent
    /// = never seen on the relays.
    pub fn online(&self) -> bool {
        match self.last_seen_secs {
            Some(secs) => secs < 3600,
            None => false,
        }
    }

    /// Human presence line for the device row: "online" or how long ago
    /// the advert was last seen.
    pub fn presence_text(&self) -> String {
        match self.last_seen_secs {
            None => "never seen".to_string(),
            Some(secs) if secs < 3600 => "online".to_string(),
            Some(secs) => {
                let age = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs().saturating_sub(secs))
                    .unwrap_or(secs);
                let text = if age < 60 * 60 {
                    format!("{}m ago", age / 60)
                } else if age < 60 * 60 * 24 {
                    format!("{}h ago", age / (60 * 60))
                } else {
                    format!("{}d ago", age / (60 * 60 * 24))
                };
                format!("last seen {text}")
            }
        }
    }
}

/// One chat bubble. Outbound messages are optimistic: `state` moves from
/// sending to sent when no failure arrives, and to failed on `ChatFailed`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChatMessage {
    pub outbound: bool,
    pub text: String,
    pub state: MessageState,
    pub at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MessageState {
    Sending,
    Sent,
    Failed,
}

/// A file transfer, inbound or outbound, live or finished.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Transfer {
    pub peer: String,
    pub path: Option<PathBuf>,
    pub outbound: bool,
    pub sent: u64,
    pub total: u64,
    pub state: TransferState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TransferState {
    Active,
    Done,
    Failed,
}

/// A live port forward: `localhost:port` is the peer's service.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Forward {
    pub peer: String,
    pub local_port: u16,
}

/// How the connection is running. `up` node mode covers chat, inbound files,
/// and presence; expose swaps the listener role (one UDP port, one role).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    Stopped,
    Node,
    Expose,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The session entity. One instance lives for the whole app run; views hold
/// `Entity<Session>` and re-render on notifications.
pub struct Session {
    client: Option<Client>,
    pub mode: RunMode,
    /// The active tab; lives here so device rows and conversations can
    /// switch tabs (the Android app navigates between fragments the same
    /// way: a device row opens its chat).
    pub tab: Tab,
    /// The conversation currently open in the Chat tab, if any.
    pub active_chat: Option<String>,
    pub my_npub: Option<String>,
    pub settings: Settings,
    pub roster: Vec<DeviceEntry>,
    /// Conversations keyed by peer npub, oldest message first.
    pub conversations: BTreeMap<String, Vec<ChatMessage>>,
    /// Unread counts keyed by peer npub.
    pub unread: BTreeMap<String, usize>,
    pub transfers: Vec<Transfer>,
    pub forwards: Vec<Forward>,
    /// Last error text to surface in the connection chip and diagnostics.
    pub last_error: Option<String>,
    /// The local TCP service exposed to peers, when mode is Expose.
    pub expose_target: Option<String>,
    /// First click of the two-step disconnect confirm.
    pub confirm_disconnect: bool,
    /// A start (node or expose) is running in the background; blocks
    /// duplicate starts.
    pub starting: bool,
}

impl Session {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let settings = Settings::load();
        let roster = std::fs::read_to_string(paths::roster_file())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        // The one identity: the mesh dir's identity.nsec. The join screen
        // shows its npub, the key package publishes under it, and the
        // client runs on it - so an invite for the shown npub always
        // reaches the running device.
        let (my_npub, last_error) = match crate::mesh::device_npub(&paths::mesh_dir()) {
            Ok(npub) => (Some(npub), None),
            Err(e) => (None, Some(format!("identity: {e}"))),
        };
        let mut session = Self {
            client: None,
            mode: RunMode::Stopped,
            tab: Tab::Devices,
            active_chat: None,
            my_npub,
            settings,
            roster,
            conversations: BTreeMap::new(),
            unread: BTreeMap::new(),
            transfers: Vec::new(),
            forwards: Vec::new(),
            last_error,
            expose_target: None,
            confirm_disconnect: false,
            starting: false,
        };
        session.sync_roster(cx);
        session.spawn_sync_timer(cx);
        // Auto-connect, like the Android foreground service coming up on
        // boot: persisted membership means the device already belongs to
        // the mesh, so relaunching goes straight to node mode.
        if !session.roster.is_empty() && session.my_npub.is_some() {
            session.start_node(cx);
        }
        session
    }

    /// Refresh the roster every two minutes for the life of the app, like
    /// the Android ClientHolder's sync timer.
    fn spawn_sync_timer(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(120))
                    .await;
                let _ = this.update(cx, |s, cx| s.sync_roster(cx));
            }
        })
        .detach();
    }

    /// Start node mode: advert published, tunnels accepted, inbound files
    /// land in the inbox, chat and presence work. This is the `noscat up`
    /// shape and what the app runs in by default once connected.
    pub fn start_node(&mut self, cx: &mut Context<Self>) {
        if self.mode == RunMode::Node || self.starting {
            return;
        }
        self.schedule_start(StartKind::Node(None), cx);
    }

    /// Start expose mode: publish a local TCP service to peers. Exclusive
    /// with node mode (one UDP port, one role), like `noscat expose`.
    pub fn start_expose(&mut self, target: std::net::SocketAddr, cx: &mut Context<Self>) {
        if self.mode == RunMode::Expose || self.starting {
            return;
        }
        self.schedule_start(StartKind::Expose(target), cx);
    }

    /// Kick off a start on the background executor: `Client::start` and the
    /// up/expose calls block for seconds on relay fetches, and the main
    /// thread never blocks (AGENTS.md). The result lands as a state update.
    fn schedule_start(&mut self, kind: StartKind, cx: &mut Context<Self>) {
        self.starting = true;
        cx.notify();
        let config = ClientConfig {
            // Same identity the mesh helpers use: one device, one npub.
            identity: IdentitySource::File(paths::mesh_dir().join("identity.nsec")),
            mesh_dir: Some(paths::mesh_dir()),
            device_name: Some(self.settings.device_name.clone()),
            ..Default::default()
        };
        cx.spawn(async move |this, cx| {
            let started = cx
                .background_executor()
                .spawn(async move {
                    let outcome: anyhow::Result<(Client, String, Receiver<Event>, RunMode)> =
                        async {
                            let mut client = Client::start(config)?;
                            let events: Receiver<Event> =
                                client.events().expect("fresh client has events");
                            let npub = client.npub().to_string();
                            match kind {
                                StartKind::Node(bridge) => {
                                    client.up(paths::inbox_dir(), bridge)?;
                                    Ok((client, npub, events, RunMode::Node))
                                }
                                StartKind::Expose(to) => {
                                    client.expose(to)?;
                                    Ok((client, npub, events, RunMode::Expose))
                                }
                            }
                        }
                        .await;
                    outcome
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.starting = false;
                match started {
                    Ok((client, npub, events, mode)) => {
                        s.client = Some(client);
                        s.my_npub = Some(npub);
                        s.mode = mode;
                        s.pump_events(events, cx);
                    }
                    Err(e) => s.last_error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Switch tabs. Any switch cancels a pending disconnect confirm.
    pub fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if self.tab != tab {
            self.confirm_disconnect = false;
        }
        self.tab = tab;
        cx.notify();
    }

    /// Stop everything: drop the client (stops the runtime and all tasks).
    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.client = None;
        self.mode = RunMode::Stopped;
        self.confirm_disconnect = false;
        cx.notify();
    }

    /// Persist a renamed device. The running client picks the new name up
    /// on its next advert; the config takes it on the next start.
    pub fn set_device_name(&mut self, name: String, cx: &mut Context<Self>) {
        if name.is_empty() || name == self.settings.device_name {
            return;
        }
        self.settings.device_name = name;
        if let Err(e) = self.settings.store() {
            self.last_error = Some(format!("saving settings: {e}"));
        }
        cx.notify();
    }

    /// Send a chat message. Optimistic: the bubble goes in immediately and
    /// shows "sent". Upstream `Client::send_chat` only emits failures
    /// (`Event::ChatFailed`), there is no delivered event yet, so there is
    /// nothing to upgrade the bubble with - see AGENTS.md.
    pub fn send_chat(&mut self, peer: &str, text: &str, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let message = ChatMessage {
            outbound: true,
            text: text.to_string(),
            state: MessageState::Sent,
            at_ms: now_ms(),
        };
        self.conversations
            .entry(peer.to_string())
            .or_default()
            .push(message);
        client.send_chat(peer, text);
        cx.notify();
    }

    /// Send a file. Progress and outcome arrive as events.
    pub fn send_file(&mut self, peer: &str, path: PathBuf, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        self.transfers.push(Transfer {
            peer: peer.to_string(),
            path: Some(path.clone()),
            outbound: true,
            sent: 0,
            total: 0,
            state: TransferState::Active,
        });
        client.send_file(path, peer);
        cx.notify();
    }

    /// Forward a local TCP port to the peer's service.
    pub fn add_forward(&mut self, peer: &str, local_port: u16, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        if let Err(e) = client.forward(
            peer,
            format!("127.0.0.1:{local_port}").parse().unwrap(),
        ) {
            self.last_error = Some(e.to_string());
        } else {
            self.forwards.push(Forward { peer: peer.to_string(), local_port });
        }
        cx.notify();
    }

    /// Share the local clipboard text with a peer.
    pub fn send_clipboard(&mut self, peer: &str, text: &str, cx: &mut Context<Self>) {
        if let Some(client) = self.client.clone() {
            client.send_clipboard(peer, text);
            cx.notify();
        }
    }
    /// Open the conversation for `peer` (clears unread).
    pub fn open_conversation(&mut self, peer: &str, cx: &mut Context<Self>) {
        self.unread.remove(peer);
        cx.notify();
    }

    /// Dismiss the last error once the user has seen it (the X on the
    /// error line in Settings). Errors otherwise stick forever and read
    /// as current when stale.
    pub fn clear_error(&mut self, cx: &mut Context<Self>) {
        self.last_error = None;
        cx.notify();
    }

    /// Refresh the roster from the relays in the background. Keeps previous
    /// names and presence aged forward when an advert fetch comes back empty.
    pub fn sync_roster(&mut self, cx: &mut Context<Self>) {
        let dir = paths::mesh_dir();
        cx.spawn(async move |this, cx| {
            let fetched = cx
                .background_executor()
                .spawn(async move { crate::mesh::members(&dir) });
            if let Ok(members) = fetched.await {
                let _ = this.update(cx, |s, cx| {
                    s.apply_members(members);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Merge a fresh member list into the roster, aging forward: a member
    /// whose fetch came back empty keeps the old name and last-seen.
    fn apply_members(&mut self, members: Vec<MeshMember>) {
        let now = now_ms();
        for m in members {
            if let Some(existing) = self.roster.iter_mut().find(|d| d.npub == m.npub) {
                if m.name.is_some() {
                    existing.name = m.name;
                }
                if m.last_seen_secs.is_some() {
                    existing.last_seen_secs = m.last_seen_secs;
                    existing.synced_at_ms = now;
                }
                existing.is_owner = m.is_owner;
            } else {
                self.roster.push(DeviceEntry {
                    npub: m.npub,
                    is_owner: m.is_owner,
                    name: m.name,
                    last_seen_secs: m.last_seen_secs,
                    synced_at_ms: now,
                });
            }
        }
        self.roster.sort_by(|a, b| {
            b.online().cmp(&a.online())
                .then_with(|| a.display_name().cmp(&b.display_name()))
        });
        self.persist_roster();
    }

    fn persist_roster(&self) {
        let dir = paths::data_dir();
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(json) = serde_json::to_string_pretty(&self.roster) {
            let _ = std::fs::write(paths::roster_file(), json);
        }
    }

    /// The one place state mutates from events. Called on the main thread.
    fn handle_event(&mut self, ev: Event, cx: &mut Context<Self>) {
        match ev {
            Event::Listening { npub, .. } => {
                self.my_npub = Some(npub);
                self.last_error = None;
                // The listener is the sync trigger: an earlier sync saw
                // nothing on the relays.
                self.sync_roster(cx);
            }
            Event::TunnelUp { .. } => {}
            Event::Incoming { .. } => {}
            Event::TunnelClosed { .. } => {}
            Event::ChatMessage { peer, text } => {
                self.conversations.entry(peer.clone()).or_default().push(ChatMessage {
                    outbound: false,
                    text,
                    state: MessageState::Sent,
                    at_ms: now_ms(),
                });
                *self.unread.entry(peer).or_default() += 1;
            }
            Event::ChatFailed { peer, error } => {
                if let Some(messages) = self.conversations.get_mut(&peer) {
                    if let Some(last) = messages
                        .iter_mut()
                        .rev()
                        .find(|m| m.outbound && m.state != MessageState::Failed)
                    {
                        last.state = MessageState::Failed;
                    }
                }
                self.last_error = Some(format!("chat to {peer} failed: {error}"));
            }
            Event::TransferProgress { peer, path, sent, total } => {
                if let Some(t) = self.transfers.iter_mut().rev().find(|t| {
                    t.peer == peer && t.outbound && t.state == TransferState::Active
                }) {
                    t.sent = sent;
                    t.total = total;
                } else {
                    self.transfers.push(Transfer {
                        peer,
                        path: Some(path),
                        outbound: true,
                        sent,
                        total,
                        state: TransferState::Active,
                    });
                }
            }
            Event::TransferDone { peer, path } => {
                // A finished send or an inbound file received by node mode.
                if let Some(t) = self.transfers.iter_mut().rev().find(|t| {
                    t.peer == peer && t.state == TransferState::Active
                }) {
                    t.path = Some(path.clone());
                    t.state = TransferState::Done;
                    t.sent = t.total;
                } else {
                    self.transfers.push(Transfer {
                        peer,
                        path: Some(path),
                        outbound: false,
                        sent: 0,
                        total: 0,
                        state: TransferState::Done,
                    });
                }
            }
            Event::TransferFailed { peer, path, error } => {
                if let Some(t) = self.transfers.iter_mut().rev().find(|t| {
                    t.peer == peer && t.state == TransferState::Active
                }) {
                    t.state = TransferState::Failed;
                } else {
                    self.transfers.push(Transfer {
                        peer,
                        path,
                        outbound: false,
                        sent: 0,
                        total: 0,
                        state: TransferState::Failed,
                    });
                }
                self.last_error = Some(format!("transfer failed: {error}"));
            }
            Event::Clipboard { peer, text } => {
                // The peer shared their clipboard: put it on ours.
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                self.conversations.entry(peer).or_default();
            }
            Event::Stopped { what, error } => {
                self.last_error = Some(match error {
                    Some(e) => format!("{what} stopped: {e}"),
                    None => format!("{what} stopped"),
                });
                if what == "node" || what == "listener" {
                    self.mode = RunMode::Stopped;
                }
            }
        }
        cx.notify();
    }
}

/// The two ways a client can start; both end in `Listening` events.
enum StartKind {
    /// `up()` node mode. The payload is reserved for a future bridge target
    /// (forwarding inbound raw streams to a local service).
    Node(Option<std::net::SocketAddr>),
    /// `expose()`: publish a local TCP service to peers.
    Expose(std::net::SocketAddr),
}

impl Session {
    /// Spawn the pump: a std thread blocks on the client's mpsc channel and
    /// forwards events into an async channel; a spawned task hops each event
    /// onto the main thread through this entity's update.
    fn pump_events(&mut self, events: Receiver<Event>, cx: &mut Context<Self>) {
        let (tx, rx) = async_channel::unbounded::<Event>();
        std::thread::Builder::new()
            .name("noscat-events".into())
            .spawn(move || {
                for ev in events {
                    if tx.send_blocking(ev).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn event pump");
        cx.spawn(async move |this: WeakEntity<Session>, cx| {
            while let Ok(ev) = rx.recv().await {
                this.update(cx, |session, cx| session.handle_event(ev, cx))?;
            }
            anyhow::Ok(())
        })
        .detach();
    }
}
