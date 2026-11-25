//! The `Session` entity: the desktop counterpart of the Android app's
//! `ClientHolder`. Owns the running `noscat_client::Client`, the event pump
//! into GPUI, and everything the tabs read: roster cache, chat history,
//! unread counts, transfer state, forwards.
//!
//! Threading rules (see AGENTS.md): events arrive on a pump thread and are
//! funneled into `Session::handle_event` on the main thread; blocking client
//! calls run on the background executor and come back as state updates.



use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use gpui_kit::gpui::{ClipboardItem, Context, WeakEntity};
use noscat_client::{Client, ClientConfig, Event, IdentitySource};

use crate::app::Tab;
use crate::mesh::MeshMember;
use crate::paths::{self, Settings, ThemePref};

/// Apply an appearance preference. System re-reads the desktop's light or
/// dark mode; the other two pin the mode. Persisted separately in settings.
pub fn apply_theme(
    pref: ThemePref,
    window: Option<&mut gpui_kit::gpui::Window>,
    cx: &mut gpui_kit::gpui::App,
) {
    use gpui_kit::component::{Theme, ThemeMode};
    match pref {
        ThemePref::System => Theme::sync_system_appearance(window, cx),
        ThemePref::Light => Theme::change(ThemeMode::Light, window, cx),
        ThemePref::Dark => Theme::change(ThemeMode::Dark, window, cx),
    }
}

/// One device in the roster: mesh membership enriched with advert name and
/// presence, aged forward across flaky relay fetches.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeviceEntry {
    pub npub: String,
    pub name: Option<String>,
    /// Seconds since the advert was last seen, from the freshest sync.
    pub last_seen_secs: Option<u64>,
    /// Monotonic timestamp (millis) of the last sync that saw this device,
    /// used to age presence forward between syncs.
    pub synced_at_ms: u64,
    /// Reachable from the internet, per its latest advert.
    #[serde(default)]
    pub reachable: bool,
    /// Data can reach it now (a direct link or a live connection). Live
    /// state only: never saved, false until the running device says so.
    #[serde(skip)]
    pub connected: bool,
}

/// What a device's dot says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// Connected: data reaches it now.
    Connected,
    /// Its advert is fresh but nothing connects: on another network with
    /// no meeting point, or a firewall in the way.
    Unreachable,
    /// No recent advert.
    Offline,
}

impl Presence {
    /// The dot color: green, amber, grey.
    pub fn color(self) -> gpui_kit::gpui::Rgba {
        gpui_kit::gpui::rgb(match self {
            Presence::Connected => 0x22c55e,
            Presence::Unreachable => 0xf59e0b,
            Presence::Offline => 0x94a3b8,
        })
    }
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

    /// Online = an advert arrived within the presence window (ten
    /// minutes; members advertise every two). Absent = not seen yet.
    pub fn online(&self) -> bool {
        match self.last_seen_secs {
            Some(secs) => secs < PRESENCE_TTL_SECS,
            None => false,
        }
    }

    /// Connected beats everything; a fresh advert without a connection is
    /// "online but unreachable".
    pub fn presence(&self) -> Presence {
        if self.connected {
            Presence::Connected
        } else if self.online() {
            Presence::Unreachable
        } else {
            Presence::Offline
        }
    }

    /// Human presence line for the device row: "connected", "online, can't
    /// connect", otherwise how long ago it was seen.
    pub fn presence_text(&self) -> String {
        match self.presence() {
            Presence::Connected => return "connected".to_string(),
            Presence::Unreachable => return "online, can't connect".to_string(),
            Presence::Offline => {}
        }
        match self.last_seen_secs {
            None => "never seen".to_string(),
            Some(secs) if secs < 180 => "online".to_string(),
            Some(secs) if secs < 3600 => format!("seen {}m ago", secs / 60),
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
    /// The member acknowledged it (`Event::ChatDelivered`): the message is
    /// in their hands.
    Delivered,
    Failed,
}

/// A file transfer, inbound or outbound, live or finished. Persisted in
/// `transfers.json`: the Sent list, and the sender of each received file.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Transfer {
    /// The client's transfer id. Ids restart with every client, so events
    /// only ever match a `live` entry.
    pub id: u64,
    /// Started by the running client (not loaded from history).
    #[serde(skip)]
    pub live: bool,
    pub peer: String,
    /// The file name as sent.
    pub name: String,
    /// Source file of a send; the final file of a finished receive.
    pub path: Option<PathBuf>,
    pub outbound: bool,
    pub sent: u64,
    pub total: u64,
    pub state: TransferState,
    /// When it started (ms since the epoch).
    pub at_ms: u64,
}

/// The short form of an npub that join screens and approve prompts show
/// to compare: "npub1abcdefg...uvwxyz".
pub fn short_id(npub: &str) -> String {
    if npub.len() > 20 {
        format!("{}...{}", &npub[..12], &npub[npub.len() - 6..])
    } else {
        npub.to_string()
    }
}

/// A device asking to join the mesh.
#[derive(Debug, Clone)]
pub struct JoinRequest {
    pub npub: String,
    /// The name it gave; anyone can claim any name, so the Devices page
    /// shows its short ID next to it to compare with the joining screen.
    pub name: Option<String>,
    /// Approve was clicked and the invite is going out.
    pub approving: bool,
}

/// Sent entries kept in the history.
pub const SENT_KEEP: usize = 100;
/// Received entries kept (only to name the sender of inbox files).
const RECEIVED_KEEP: usize = 500;

/// Messages kept per conversation in `chats.json`.
const CHAT_KEEP: usize = 500;

fn load_chats() -> BTreeMap<String, Vec<ChatMessage>> {
    let mut c: BTreeMap<String, Vec<ChatMessage>> = std::fs::read_to_string(paths::chats_file())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    // A send that never resolved before the app stopped did not go out.
    for m in c.values_mut().flatten() {
        if m.state == MessageState::Sending {
            m.state = MessageState::Failed;
        }
    }
    c
}

/// Show a desktop notification. Off the UI thread: it talks to the
/// notification daemon over D-Bus, and a missing daemon must not stall.
fn notify(title: String, body: String) {
    std::thread::spawn(move || {
        let _ = notify_rust::Notification::new()
            .appname("noscat")
            .summary(&title)
            .body(&body)
            .show();
    });
}

fn load_transfers() -> Vec<Transfer> {
    let mut v: Vec<Transfer> = std::fs::read_to_string(paths::transfers_file())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    // Whatever was running when the app last stopped did not finish.
    for t in &mut v {
        if t.state == TransferState::Active {
            t.state = TransferState::Failed;
        }
    }
    v
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TransferState {
    Active,
    Done,
    /// The receive ended without a verified whole file (client 0.4.1's
    /// `Event::TransferPartial`): the kept `.noscat-part` is a resume
    /// seed, not a usable file. Never shown as done.
    Partial,
    Failed,
    /// Cancelled by this side (`cancel_transfer`): the receiver keeps its
    /// resumable partial, and a later send of the same file resumes.
    Cancelled,
}

/// A live port forward: `localhost:port` is the peer's service. `id` is the
/// client's handle for `cancel_forward`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Forward {
    pub id: u64,
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

/// A member counts as online while its latest advert is this fresh
/// (noscat-core's presence window).
const PRESENCE_TTL_SECS: u64 = 600;

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
    /// What the pairing QR encodes: `noscat:<npub>`, plus the mesh's relays
    /// when it is private (from the running client; `None` before it runs).
    pub pairing_uri: Option<String>,
    /// The UDP port the node actually bound (ephemeral, from the
    /// Listening event); shown in diagnostics and cleared on stop.
    pub listen_port: Option<u16>,
    pub settings: Settings,
    pub roster: Vec<DeviceEntry>,
    /// Conversations keyed by peer npub, oldest message first.
    pub conversations: BTreeMap<String, Vec<ChatMessage>>,
    /// Unread counts keyed by peer npub.
    pub unread: BTreeMap<String, usize>,
    pub transfers: Vec<Transfer>,
    pub forwards: Vec<Forward>,
    /// Devices asking to join (they scanned or copied this device's ID),
    /// waiting for Approve or Decline on the Devices page.
    pub join_requests: Vec<JoinRequest>,
    /// The window has focus; messages for the open conversation then need
    /// no notification. Kept by the app shell.
    pub window_active: bool,
    /// Last error text to surface in the connection chip and diagnostics.
    pub last_error: Option<String>,
    /// The local TCP service exposed to peers, when mode is Expose.
    pub expose_target: Option<String>,
    /// First click of the two-step disconnect confirm.
    pub confirm_disconnect: bool,
    /// First click of the two-step forget confirm.
    pub confirm_forget: bool,
    /// A forget (leave, then wipe the device) is running.
    pub forgetting: bool,
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
        // client runs on it, so an invite for the shown npub always
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
            pairing_uri: None,
            listen_port: None,
            settings,
            roster,
            conversations: load_chats(),
            unread: BTreeMap::new(),
            transfers: load_transfers(),
            forwards: Vec::new(),
            join_requests: Vec::new(),
            window_active: true,
            last_error,
            expose_target: None,
            confirm_disconnect: false,
            confirm_forget: false,
            forgetting: false,
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

    /// Presence ticks every 30 seconds while the client runs: the dot
    /// follows the advert watcher's live feed (members that stop
    /// advertising fall out of "online" when their advert's hour is up).
    /// Harmless if respawned: a listener restart starts a fresh one, and
    /// a tick with no client does nothing.
    fn spawn_presence_timer(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(30))
                    .await;
                let _ = this.update(cx, |s, cx| s.refresh_presence(cx));
            }
        })
        .detach();
    }

    /// Start node mode: adverts sent, member streams accepted, inbound files
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
            // Port 0 = the OS picks a free UDP port (the real bound port
            // comes back in the Listening event). Several noscat nodes can
            // share a machine (a CLI node beside the desktop app, and
            // later one per app), and peers dial whatever the advert
            // carries, so a fixed port buys nothing. A reachable device
            // needs the fixed port the router forwards: 41641.
            listen: Some(
                if self.settings.reachable { "0.0.0.0:41641" } else { "0.0.0.0:0" }
                    .parse()
                    .expect("static listen addr"),
            ),
            // Reachable: the public address is learned by STUN.
            reachable: self.settings.reachable.then_some(None),
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
                    Err(e) => {
                        let text = e.to_string();
                        // The common cause on desktop: a leftover `noscat
                        // up` (CLI node) holding UDP 41641. Name the fix,
                        // not the errno.
                        if text.contains("already in use") {
                            s.last_error = Some(
                                "Another noscat node is already running on this \
                                 machine and holds the network port. Quit it \
                                 (look for a terminal running `noscat up`), \
                                 then connect again."
                                    .into(),
                            );
                        } else {
                            s.last_error = Some(text);
                        }
                    }
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
            self.confirm_forget = false;
        }
        self.tab = tab;
        // Back on a conversation that stayed open: it is read now.
        if tab == Tab::Chat {
            if let Some(peer) = &self.active_chat {
                self.unread.remove(peer);
            }
        }
        cx.notify();
    }

    /// Turn "reachable from the internet" on or off. Saved, and a running
    /// device restarts in the same mode to bind the new port.
    pub fn set_reachable(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.settings.reachable == on || self.starting {
            return;
        }
        self.settings.reachable = on;
        if let Err(e) = self.settings.store() {
            self.last_error = Some(format!("could not save settings: {e}"));
        }
        let mode = self.mode;
        if mode == RunMode::Stopped {
            cx.notify();
            return;
        }
        let expose = self.expose_target.as_deref().and_then(|t| t.parse::<std::net::SocketAddr>().ok());
        self.client = None;
        self.pairing_uri = None;
        self.mode = RunMode::Stopped;
        self.starting = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            // The dropped client releases the device dir and its port.
            cx.background_executor()
                .spawn(async move {
                    for _ in 0..50 {
                        if Client::running(&paths::mesh_dir()).is_none() {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.starting = false;
                match (mode, expose) {
                    (RunMode::Expose, Some(target)) => s.start_expose(target, cx),
                    _ => s.start_node(cx),
                }
            });
        })
        .detach();
    }

    /// Go offline: drop the client (stops the runtime and all tasks). The
    /// device stays a member; connecting again resumes it.
    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.client = None;
        self.pairing_uri = None;
        for d in &mut self.roster {
            d.connected = false;
        }
        self.mode = RunMode::Stopped;
        self.confirm_disconnect = false;
        self.confirm_forget = false;
        cx.notify();
    }

    /// Leave the mesh for good: the other members remove this device, then
    /// the device directory (identity and mesh state) and the cached roster
    /// are deleted, so joining again starts as a new device. Received files
    /// stay. If leaving fails (no relay reachable), nothing is deleted.
    pub fn forget(&mut self, cx: &mut Context<Self>) {
        if self.forgetting {
            return;
        }
        self.forgetting = true;
        self.confirm_forget = false;
        self.confirm_disconnect = false;
        cx.notify();
        let dir = paths::mesh_dir();
        cx.spawn(async move |this, cx| {
            let left = cx
                .background_executor()
                .spawn({
                    let dir = dir.clone();
                    async move { crate::mesh::leave(&dir, 30) }
                })
                .await;
            if let Err(e) = left {
                let _ = this.update(cx, |s, cx| {
                    s.forgetting = false;
                    s.last_error = Some(format!(
                        "Could not leave the mesh ({e}). Nothing was deleted; try again when online."
                    ));
                    cx.notify();
                });
                return;
            }
            // Stop the device before deleting its directory.
            let _ = this.update(cx, |s, cx| {
                s.client = None;
                s.pairing_uri = None;
                s.mode = RunMode::Stopped;
                cx.notify();
            });
            let wiped = cx
                .background_executor()
                .spawn({
                    let dir = dir.clone();
                    async move { crate::mesh::wipe(&dir) }
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.forgetting = false;
                s.roster.clear();
                let _ = std::fs::remove_file(paths::roster_file());
                s.conversations.clear();
                let _ = std::fs::remove_file(paths::chats_file());
                s.unread.clear();
                s.active_chat = None;
                s.transfers.clear();
                let _ = std::fs::remove_file(paths::transfers_file());
                s.forwards.clear();
                s.join_requests.clear();
                s.expose_target = None;
                s.listen_port = None;
                s.tab = Tab::Devices;
                // The next join runs on a new identity (created now so the
                // join screen can show it).
                s.my_npub = crate::mesh::device_npub(&dir).ok();
                s.last_error = wiped.err().map(|e| {
                    format!("Left the mesh, but could not delete the device data: {e}")
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// Rename this device: saved for the next start, and the running
    /// client tells the members now (they see it within seconds).
    pub fn set_device_name(&mut self, name: String, cx: &mut Context<Self>) {
        if name.is_empty() || name == self.settings.device_name {
            return;
        }
        self.settings.device_name = name.clone();
        if let Err(e) = self.settings.store() {
            self.last_error = Some(format!("could not save settings: {e}"));
        }
        if let Some(client) = self.client.clone() {
            // Blocking: it publishes the advert.
            cx.background_executor()
                .spawn(async move { client.set_device_name(&name) })
                .detach();
        }
        cx.notify();
    }

    /// Persist the appearance preference. Applying it needs the window
    /// (theme changes refresh every window), so the view calls
    /// `state::apply_theme` from its listener.
    pub fn set_theme(&mut self, pref: ThemePref, cx: &mut Context<Self>) {
        if self.settings.theme == pref {
            return;
        }
        self.settings.theme = pref;
        if let Err(e) = self.settings.store() {
            self.last_error = Some(format!("could not save settings: {e}"));
        }
        cx.notify();
    }

    /// Send a chat message. Optimistic: the bubble goes in at once as
    /// "sending"; `ChatDelivered` or `ChatFailed` settles it.
    pub fn send_chat(&mut self, peer: &str, text: &str, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let message = ChatMessage {
            outbound: true,
            text: text.to_string(),
            state: MessageState::Sending,
            at_ms: now_ms(),
        };
        self.conversations
            .entry(peer.to_string())
            .or_default()
            .push(message);
        self.persist_chats();
        client.send_chat(peer, text);
        cx.notify();
    }

    /// Send a message that was not delivered again: the failed one goes
    /// and its text is sent as a new message. Never automatic: "not
    /// delivered" can mean the member got it but the acknowledgement was
    /// lost.
    pub fn retry_chat(&mut self, peer: &str, index: usize, cx: &mut Context<Self>) {
        if self.client.is_none() {
            return;
        }
        let Some(messages) = self.conversations.get_mut(peer) else { return };
        if messages.get(index).is_none_or(|m| !m.outbound || m.state != MessageState::Failed) {
            return;
        }
        let text = messages.remove(index).text;
        self.send_chat(peer, &text, cx);
    }

    /// Remove one message from this device's history (only here).
    pub fn delete_chat(&mut self, peer: &str, index: usize, cx: &mut Context<Self>) {
        let Some(messages) = self.conversations.get_mut(peer) else { return };
        if index < messages.len() {
            messages.remove(index);
            self.persist_chats();
            cx.notify();
        }
    }

    /// Send a file. Progress and outcome arrive as events.
    pub fn send_file(&mut self, peer: &str, path: PathBuf, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let total = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let id = client.send_file(path.clone(), peer);
        self.transfers.push(Transfer {
            id,
            live: true,
            peer: peer.to_string(),
            name,
            path: Some(path),
            outbound: true,
            sent: 0,
            total,
            state: TransferState::Active,
            at_ms: now_ms(),
        });
        self.persist_transfers();
        cx.notify();
    }

    /// Stop an active send; the receiver keeps its partial, so sending the
    /// same file again resumes.
    pub fn cancel_send(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else { return };
        if let Some(t) = self.transfers.iter().find(|t| t.live && t.id == id && t.outbound) {
            if let Some(path) = t.path.clone() {
                client.cancel_transfer(&t.peer, &path);
            }
        }
        cx.notify();
    }

    /// Send a finished (failed, stopped or done) entry's file again.
    pub fn resend(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(t) = self.transfers.get(index).cloned() else { return };
        if let (true, Some(path)) = (t.outbound, t.path) {
            self.send_file(&t.peer, path, cx);
        }
    }

    /// Clear the Sent history (active sends stay).
    pub fn clear_sent(&mut self, cx: &mut Context<Self>) {
        self.transfers.retain(|t| !t.outbound || t.state == TransferState::Active);
        self.persist_transfers();
        cx.notify();
    }

    /// Save the history, keeping the newest [`SENT_KEEP`] sends and
    /// [`RECEIVED_KEEP`] receives.
    fn persist_transfers(&mut self) {
        for (outbound, keep) in [(true, SENT_KEEP), (false, RECEIVED_KEEP)] {
            let n = self.transfers.iter().filter(|t| t.outbound == outbound).count();
            let mut drop = n.saturating_sub(keep);
            self.transfers.retain(|t| {
                if drop > 0 && t.outbound == outbound && t.state != TransferState::Active {
                    drop -= 1;
                    false
                } else {
                    true
                }
            });
        }
        let _ = std::fs::create_dir_all(paths::data_dir());
        if let Ok(json) = serde_json::to_string(&self.transfers) {
            let _ = std::fs::write(paths::transfers_file(), json);
        }
    }

    /// Save the chats (the newest [`CHAT_KEEP`] per conversation).
    fn persist_chats(&self) {
        let trimmed: BTreeMap<&String, &[ChatMessage]> = self
            .conversations
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| (k, &v[v.len().saturating_sub(CHAT_KEEP)..]))
            .collect();
        let _ = std::fs::create_dir_all(paths::data_dir());
        if let Ok(json) = serde_json::to_string(&trimmed) {
            let _ = std::fs::write(paths::chats_file(), json);
        }
    }

    /// The live entry for a client transfer id.
    fn live_transfer(&mut self, id: u64) -> Option<&mut Transfer> {
        self.transfers.iter_mut().rev().find(|t| t.live && t.id == id)
    }

    /// The roster display name for a peer, or "device".
    pub fn peer_name(&self, npub: &str) -> String {
        self.roster
            .iter()
            .find(|d| d.npub == npub)
            .map(|d| d.display_name())
            .unwrap_or_else(|| "device".to_string())
    }

    /// Replace any npub in a user-facing message with the device's name:
    /// identity strings are not user information, even in errors.
    fn scrub_npubs(&self, text: &str) -> String {
        text.split_whitespace()
            .map(|word| {
                let core = word.trim_end_matches(|c: char| ",.;:!?)(".contains(c));
                if core.starts_with("npub1") {
                    let trail = &word[core.len()..];
                    if self.roster.iter().any(|d| d.npub == core) {
                        format!("{}{}", self.peer_name(core), trail)
                    } else {
                        format!("another device{trail}")
                    }
                } else {
                    word.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Client error text as a human reads it: the wire's jargon becomes a
    /// sentence about what to check.
    fn human_reason(error: &str) -> String {
        let lower = error.to_lowercase();
        if lower.contains("handshake") {
            "no secure handshake: the device may be offline or out of reach".into()
        } else if lower.contains("malformed") {
            "the reply did not make sense".into()
        } else if lower.contains("timeout") || lower.contains("timed out") {
            "it did not respond in time".into()
        } else {
            error.to_string()
        }
    }

    /// Forward a local TCP port to the peer's service. `forward` returns
    /// the id (client 0.4.0) the confirming `Event::ForwardStopped` carries.
    pub fn add_forward(&mut self, peer: &str, local_port: u16, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let id = client.forward(peer, format!("127.0.0.1:{local_port}").parse().unwrap());
        self.forwards.push(Forward { id, peer: peer.to_string(), local_port });
        cx.notify();
    }

    /// Stop a forward: the accept loop ends, live connections finish. The
    /// row stays until the confirming `Event::ForwardStopped` arrives.
    pub fn stop_forward(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(client) = self.client.clone() {
            client.cancel_forward(id);
            cx.notify();
        }
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

    /// Refresh the roster from the running client, which follows the mesh
    /// live (joins and removals while this device was offline arrive when
    /// it reconnects). Names and presence come from in-group adverts.
    /// Invite a device that asked to join. Blocking work on the background
    /// executor; the request goes when the invite succeeds.
    pub fn approve_join(&mut self, npub: String, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else { return };
        let Some(r) = self.join_requests.iter_mut().find(|r| r.npub == npub) else { return };
        if r.approving {
            return;
        }
        r.approving = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let target = npub.clone();
            let res = cx
                .background_executor()
                .spawn(async move { client.invite(&target) })
                .await;
            let _ = this.update(cx, |s, cx| {
                match res {
                    Ok(()) => {
                        s.join_requests.retain(|r| r.npub != npub);
                        s.sync_roster(cx);
                    }
                    Err(e) => {
                        if let Some(r) = s.join_requests.iter_mut().find(|r| r.npub == npub) {
                            r.approving = false;
                        }
                        s.last_error = Some(format!("invite failed: {}", Self::human_reason(&e.to_string())));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Ignore a join request. The device can ask again.
    pub fn decline_join(&mut self, npub: &str, cx: &mut Context<Self>) {
        self.join_requests.retain(|r| r.npub != npub);
        cx.notify();
    }

    pub fn sync_roster(&mut self, cx: &mut Context<Self>) {
        // The running device is always caught up (it follows the group
        // live); with none running there is nothing newer than the cached
        // roster, and starting a device here would race `start_node` for
        // the mesh directory.
        let Some(client) = self.client.clone() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let (members, uri) = cx
                .background_executor()
                .spawn(async move { (crate::mesh::members_of(&client), client.pairing_uri()) })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.apply_members(members);
                s.pairing_uri = Some(uri);
                cx.notify();
            });
        })
        .detach();
    }

    /// Pull liveness from the client's own advert watcher (0.4.1): `up`
    /// already watches advert refreshes live, so the dot stops being a
    /// two-minute-old relay poll. `None` from the watcher only means "not
    /// seen since the watcher started": the previous value ages forward,
    /// never backward.
    pub fn refresh_presence(&mut self, cx: &mut Context<Self>) {
        // The member list carries presence and connection state together.
        self.sync_roster(cx);
    }

    /// Merge a fresh member list into the roster, aging forward: a member
    /// whose fetch came back empty keeps the old name and last-seen. The
    /// member list itself is the device's local group state (not a relay
    /// fetch), so a member absent from it truly left or was removed: the
    /// row goes.
    fn apply_members(&mut self, members: Vec<MeshMember>) {
        let now = now_ms();
        // Departed members (removed by another member, left on their own) are
        // gone from the group; keep the list from haunting the UI. The
        // list is the device's local group state, not a flaky relay fetch,
        // so absence is truth.
        self.roster
            .retain(|d| members.iter().any(|m| m.npub == d.npub));
        for m in members {
            if let Some(existing) = self.roster.iter_mut().find(|d| d.npub == m.npub) {
                if m.name.is_some() {
                    existing.name = m.name;
                }
                if m.last_seen_secs.is_some() {
                    existing.last_seen_secs = m.last_seen_secs;
                    existing.synced_at_ms = now;
                    existing.reachable = m.reachable;
                }
                existing.connected = m.connected;
            } else {
                self.roster.push(DeviceEntry {
                    npub: m.npub,
                    name: m.name,
                    last_seen_secs: m.last_seen_secs,
                    synced_at_ms: now,
                    reachable: m.reachable,
                    connected: m.connected,
                });
            }
        }
        // The order every noscat UI uses: this device first, then by
        // name (case-insensitive), then devices with no name yet, ties by
        // npub. Presence never reorders the list (the dot shows it).
        let me = self.my_npub.clone();
        self.roster.sort_by(|a, b| {
            let key = |d: &DeviceEntry| {
                let name = d.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).map(str::to_lowercase);
                (Some(&d.npub) != me.as_ref(), name.is_none(), name, d.npub.clone())
            };
            key(a).cmp(&key(b))
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
            Event::Listening { npub, addr } => {
                self.my_npub = Some(npub);
                self.listen_port = Some(addr.port());
                self.last_error = None;
                // The listener is the sync trigger: an earlier sync saw
                // nothing on the relays.
                self.sync_roster(cx);
                self.refresh_presence(cx);
                self.spawn_presence_timer(cx);
            }
            Event::TunnelUp { .. } => {}
            Event::TunnelClosed { .. } => {}
            Event::MeshJoined { .. } | Event::Roster { .. } => self.sync_roster(cx),
            Event::MemberSeen { peer, name } => {
                // The advert carries the member's current name: a rename
                // shows everywhere now, not at the next roster sync.
                if let Some(name) = name.filter(|n| !n.trim().is_empty()) {
                    if let Some(d) = self.roster.iter_mut().find(|d| d.npub == peer) {
                        if d.name.as_deref() != Some(name.as_str()) {
                            d.name = Some(name);
                            self.persist_roster();
                        }
                    }
                }
                self.refresh_presence(cx);
            }
            Event::MeshChangeUndone => {
                self.last_error = Some(
                    "Another device changed the mesh at the same moment and won; \
                     try again."
                        .into(),
                );
            }
            Event::MeshLeft => {
                self.roster.clear();
                self.persist_roster();
                self.last_error = Some("This device is no longer in the mesh.".into());
            }
            // Inviting from the desktop app needs an approve prompt (not
            // built yet); the mesh's inviting device handles requests.
            Event::JoinRequest { peer, name } => {
                // A member already, or asked before: nothing new to show
                // (keep the newest name).
                if !self.roster.iter().any(|d| d.npub == peer) {
                    if let Some(r) = self.join_requests.iter_mut().find(|r| r.npub == peer) {
                        r.name = name;
                    } else {
                        let who = name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "A device".into());
                        notify(
                            format!("{who} wants to join your mesh"),
                            "Approve or decline on the Devices page.".into(),
                        );
                        self.join_requests.push(JoinRequest { npub: peer, name, approving: false });
                    }
                }
            }
            Event::ChatMessage { peer, text } => {
                self.conversations.entry(peer.clone()).or_default().push(ChatMessage {
                    outbound: false,
                    text,
                    state: MessageState::Sent,
                    at_ms: now_ms(),
                });
                self.persist_chats();
                // Unread only when that conversation is not on screen; a
                // notification unless the user is looking at it.
                let on_screen = self.tab == Tab::Chat && self.active_chat.as_deref() == Some(peer.as_str());
                if !(on_screen && self.window_active) {
                    let text = self.conversations[&peer].last().map(|m| m.text.clone()).unwrap_or_default();
                    notify(self.peer_name(&peer), text);
                }
                if !on_screen {
                    *self.unread.entry(peer).or_default() += 1;
                }
            }
            Event::ChatDelivered { peer } => {
                // The member acknowledged it: the newest outbound
                // message still pending is in their hands.
                if let Some(messages) = self.conversations.get_mut(&peer) {
                    if let Some(last) = messages
                        .iter_mut()
                        .rev()
                        .find(|m| m.outbound && matches!(m.state, MessageState::Sending | MessageState::Sent))
                    {
                        last.state = MessageState::Delivered;
                    }
                }
                self.persist_chats();
            }
            Event::ChatFailed { peer, error } => {
                if let Some(messages) = self.conversations.get_mut(&peer) {
                    if let Some(last) = messages
                        .iter_mut()
                        .rev()
                        .find(|m| m.outbound && matches!(m.state, MessageState::Sending | MessageState::Sent))
                    {
                        last.state = MessageState::Failed;
                    }
                }
                self.persist_chats();
                self.last_error = Some(self.scrub_npubs(&format!(
                    "could not chat with {}: {}",
                    self.peer_name(&peer),
                    Self::human_reason(&error)
                )));
            }
            Event::TransferStarted { id, peer, name, size, incoming, path } => {
                if let Some(t) = self.live_transfer(id) {
                    t.total = size;
                    t.name = name;
                } else {
                    self.transfers.push(Transfer {
                        id,
                        live: true,
                        peer,
                        name,
                        path,
                        outbound: !incoming,
                        sent: 0,
                        total: size,
                        state: TransferState::Active,
                        at_ms: now_ms(),
                    });
                    self.persist_transfers();
                }
            }
            Event::TransferProgress { id, sent, total, .. } => {
                if let Some(t) = self.live_transfer(id) {
                    t.sent = sent;
                    t.total = total;
                }
            }
            Event::TransferDone { id, path, peer } => {
                // A finished send, or a received file (its final name).
                let mut received = false;
                if let Some(t) = self.live_transfer(id) {
                    received = !t.outbound;
                    t.path = Some(path.clone());
                    t.state = TransferState::Done;
                    t.sent = t.total;
                }
                if received && !self.window_active {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    notify(format!("Received {name}"), format!("from {}", self.peer_name(&peer)));
                }
                self.persist_transfers();
            }
            Event::TransferPartial { id, path, .. } => {
                // The receive ended without a verified whole file: what is
                // on disk is a `.noscat-part` resume seed, not the file.
                if let Some(t) = self.live_transfer(id) {
                    t.path = Some(path);
                    t.state = TransferState::Partial;
                }
                self.persist_transfers();
            }
            Event::TransferCancelled { id, .. } => {
                // Our own cancel (a later send of the same file resumes).
                if let Some(t) = self.live_transfer(id) {
                    t.state = TransferState::Cancelled;
                }
                self.persist_transfers();
            }
            Event::TransferFailed { id, error, .. } => {
                if let Some(t) = self.live_transfer(id) {
                    t.state = TransferState::Failed;
                }
                self.persist_transfers();
                self.last_error = Some(self.scrub_npubs(&format!(
                    "file transfer failed: {}",
                    Self::human_reason(&error)
                )));
            }
            Event::Clipboard { peer, text } => {
                // The peer shared their clipboard: put it on ours.
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                self.conversations.entry(peer).or_default();
            }
            Event::ForwardStopped { id, error } => {
                // The confirming end of a cancel, or a failure: either way
                // the row goes. A failed bind or accept explains itself.
                let peer = self
                    .forwards
                    .iter()
                    .find(|f| f.id == id)
                    .map(|f| f.peer.clone());
                self.forwards.retain(|f| f.id != id);
                if let (Some(e), Some(peer)) = (error, peer) {
                    self.last_error = Some(self.scrub_npubs(&format!(
                        "the forward to {} stopped unexpectedly: {}",
                        self.peer_name(&peer),
                        Self::human_reason(&e)
                    )));
                }
            }
            Event::Stopped { what, error } => {
                let what = match what.as_ref() {
                    "node" => "the connection",
                    "listener" => "the listener",
                    "expose" => "the exposed service",
                    "forward" => "a port forward",
                    "clipboard" => "a clipboard share",
                    "bridge" => "the bridge",
                    other => other,
                };
                self.last_error = Some(self.scrub_npubs(&match error {
                    Some(e) => format!("{what} stopped unexpectedly: {}", Self::human_reason(&e)),
                    None => format!("{what} stopped unexpectedly"),
                }));
                if what == "node" || what == "listener" {
                    self.mode = RunMode::Stopped;
                    self.listen_port = None;
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
