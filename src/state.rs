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
use nostr::prelude::PublicKey;
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

    /// Human presence line for the device row: "online" when the advert
    /// feed is fresh, otherwise how long ago it was seen.
    pub fn presence_text(&self) -> String {
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
            listen_port: None,
            settings,
            roster,
            conversations: BTreeMap::new(),
            unread: BTreeMap::new(),
            transfers: Vec::new(),
            forwards: Vec::new(),
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
    /// Harmless if respawned - a listener restart starts a fresh one, and
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
            // share a machine - a CLI node beside the desktop app, and
            // later one per app - and peers dial whatever the advert
            // carries, so a fixed port buys nothing.
            listen: Some("0.0.0.0:0".parse().expect("static listen addr")),
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
        cx.notify();
    }

    /// Go offline: drop the client (stops the runtime and all tasks). The
    /// device stays a member; connecting again resumes it.
    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.client = None;
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
                s.unread.clear();
                s.active_chat = None;
                s.transfers.clear();
                s.forwards.clear();
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

    /// Persist a renamed device. The running client picks the new name up
    /// on its next advert; the config takes it on the next start.
    pub fn set_device_name(&mut self, name: String, cx: &mut Context<Self>) {
        if name.is_empty() || name == self.settings.device_name {
            return;
        }
        self.settings.device_name = name;
        if let Err(e) = self.settings.store() {
            self.last_error = Some(format!("could not save settings: {e}"));
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

    /// The roster display name for a peer, or "device".
    fn peer_name(&self, npub: &str) -> String {
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
            "no secure handshake - the device may be offline or out of reach".into()
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
    pub fn sync_roster(&mut self, cx: &mut Context<Self>) {
        // The running device is always caught up (it follows the group
        // live); with none running there is nothing newer than the cached
        // roster, and starting a device here would race `start_node` for
        // the mesh directory.
        let Some(client) = self.client.clone() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let members = cx
                .background_executor()
                .spawn(async move { crate::mesh::members_of(&client) })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.apply_members(members);
                cx.notify();
            });
        })
        .detach();
    }

    /// Pull liveness from the client's own advert watcher (0.4.1): `up`
    /// already watches advert refreshes live, so the dot stops being a
    /// two-minute-old relay poll. `None` from the watcher only means "not
    /// seen since the watcher started" - the previous value ages forward,
    /// never backward.
    pub fn refresh_presence(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        // The watcher speaks hex; the roster stores bech32 npubs.
        let keys: Vec<String> = self.roster.iter().map(|d| d.npub.clone()).collect();
        cx.spawn(async move |this, cx| {
            let ages = cx.background_executor().spawn(async move {
                keys.into_iter()
                    .map(|npub| {
                        let age = PublicKey::parse(&npub)
                            .map(|k| client.member_age_secs(&k.to_hex()))
                            .unwrap_or(None);
                        (npub, age)
                    })
                    .collect::<Vec<_>>()
            })
            .await;
            let _ = this.update(cx, |s, cx| {
                for (npub, age) in ages {
                    if let (Some(d), Some(secs)) =
                        (s.roster.iter_mut().find(|d| d.npub == npub), age)
                    {
                        d.last_seen_secs = Some(secs);
                        d.synced_at_ms = now_ms();
                    }
                }
                s.roster.sort_by(|a, b| {
                    b.online().cmp(&a.online())
                        .then_with(|| a.display_name().cmp(&b.display_name()))
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// Merge a fresh member list into the roster, aging forward: a member
    /// whose fetch came back empty keeps the old name and last-seen. The
    /// member list itself is the device's local group state (not a relay
    /// fetch), so a member absent from it truly left or was removed - the
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
                }
            } else {
                self.roster.push(DeviceEntry {
                    npub: m.npub,
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
            Event::MemberSeen { .. } => self.refresh_presence(cx),
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
            Event::JoinRequest { .. } => {}
            Event::ChatMessage { peer, text } => {
                self.conversations.entry(peer.clone()).or_default().push(ChatMessage {
                    outbound: false,
                    text,
                    state: MessageState::Sent,
                    at_ms: now_ms(),
                });
                *self.unread.entry(peer).or_default() += 1;
            }
            Event::ChatDelivered { peer } => {
                // The member acknowledged it: the newest outbound
                // message not already failed is in their hands.
                if let Some(messages) = self.conversations.get_mut(&peer) {
                    if let Some(last) = messages
                        .iter_mut()
                        .rev()
                        .find(|m| m.outbound && m.state != MessageState::Failed)
                    {
                        last.state = MessageState::Delivered;
                    }
                }
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
                self.last_error = Some(self.scrub_npubs(&format!(
                    "could not chat with {}: {}",
                    self.peer_name(&peer),
                    Self::human_reason(&error)
                )));
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
            Event::TransferPartial { peer, path } => {
                // The receive ended without a verified whole file: what is
                // on disk is a `.noscat-part` resume seed, not the file.
                // Must never read as done - the Android Files list treats
                // this as its own state too.
                if let Some(t) = self.transfers.iter_mut().rev().find(|t| {
                    t.peer == peer && t.state == TransferState::Active
                }) {
                    t.path = Some(path.clone());
                    t.state = TransferState::Partial;
                } else {
                    self.transfers.push(Transfer {
                        peer,
                        path: Some(path),
                        outbound: false,
                        sent: 0,
                        total: 0,
                        state: TransferState::Partial,
                    });
                }
            }
            Event::TransferCancelled { peer, path } => {
                // Our own cancel (a later send of the same file resumes).
                // Not a failure: the row reports the cancellation neutrally.
                if let Some(t) = self.transfers.iter_mut().rev().find(|t| {
                    t.peer == peer && t.state == TransferState::Active
                }) {
                    t.path = Some(path.clone());
                    t.state = TransferState::Cancelled;
                } else {
                    self.transfers.push(Transfer {
                        peer,
                        path: Some(path),
                        outbound: true,
                        sent: 0,
                        total: 0,
                        state: TransferState::Cancelled,
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
