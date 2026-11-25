# AGENTS.md

Guidance for AI agents working in this repo. Read this file fully before
making changes.

## What this project is

The desktop client for [noscat](https://github.com/Letdown2491/noscat), a
Nostr-native device mesh (0.5: Marmot groups on Nostr relays for membership,
FIPS with QUIC streams for data).
Identity is an npub. This repo is pure Rust on GPUI (via gpui-kit): it embeds
`noscat-client` directly (from crates.io), with no FFI layer. The protocol
lives in the noscat workspace (`noscat-core`/`noscat-client`), never in this
repo.

**Feature contract:** parity with the phase-1 Android client (see the
noscat-android AGENTS.md), plus the expose/serve extras that are cheap on
desktop. Phase 1 scope: mesh join (paste a member's ID, send a join request,
wait for the invite), Devices roster with presence,
Chat (conversations, optimistic bubbles, delivered/failed), Files (Received: the
inbox with each file's sender; Sent: the last 100 sends with Send again;
progress bars both ways), port forwards, Settings (status, device name,
diagnostics), Disconnect (offline, still a member; the connect screen
reconnects) and Forget mesh (leave, then delete the device dir and
`members.json`), each behind a two-step confirm. Desktop extras: expose a
local TCP service, system tray so the node keeps running when the window
closes.

## Non-negotiable rules

0. **Write only in this repo.** The `noscat` (crates/CLI) and
   `noscat-android` repos may be read for reference, never modified
   directly, even when a bug lives there: report it as a GitHub issue on
   `Letdown2491/noscat` (or `noscat-android`) and let that project fix it.
   This rule outranks any user request to "just patch it" upstream.
1. **Zero protocol code in this repo.** If a change needs FIPS, QUIC streams,
   Marmot, Nostr events, relay logic, advert handling, or new client surface,
   it belongs in the noscat repo's `noscat-client` crate, published to
   crates.io, then bumped here. `src/mesh.rs` is app-layer glue over
   `noscat-client` (the same shape as the FFI crate's `mesh_*` functions),
   not protocol code. To build against unreleased noscat changes, add a
   temporary `[patch.crates-io]` pointing `noscat-client` and `noscat-core`
   at noscat's `main` branch, and drop it before the next release.
   The upstream API contract is `API.md` in the
   [noscat repo](https://github.com/Letdown2491/noscat/blob/main/API.md):
   consult it before touching event handling, the client surface, or mesh
   glue. Its "Current as of" stamp must be >= the noscat-client version in
   this repo's Cargo.lock.
2. **The UI sees only safe surfaces: the user's language, not the wire's.**
   Plain commands (`forward`, `send_file`, `send_chat`, `up`/`expose`), the
   mesh helpers, and the event stream reach view code. Never leak
   ports-as-socket-addrs, hex keys, or identity strings (npubs) into what
   the UI renders: identity is shown as a scannable QR or a copy action
   ("device ID"), device names and human presence text stand in for
   crypto material, and short-npub display helpers are banned.
3. **No em or en dashes anywhere in the repo, and no hyphen standing in
   for one** (" - " between words): use a colon, semicolon, comma or
   parentheses. Applies to code comments, UI text, docs, commit messages,
   everything.
4. **Events arrive on a background thread.** The event pump thread forwards
   `noscat_client::Event` values into the GPUI executor; state mutation only
   happens inside `Session::update` on the main thread. Views never touch the
   client directly, only the `Session` entity.
5. **No blocking client calls on the main thread.** `Client::start`,
   `Client::connect`, the mesh helpers, and `up`/`expose` block for seconds
   (relays, path setup). They run on the background executor; results come
   back as state updates.
6. **Client state survives view churn.** The roster persists to
   `members.json` so startup renders the device list instantly; presence
   ages forward between updates from the client instead of flashing live
   members offline.
7. **GitHub pushes go over SSH** (`git@github.com:...`), not HTTPS.

## Layout

- `src/main.rs`: GPUI bootstrap, window options, tray lifecycle hookup.
- `src/app.rs`: the root view: connection chip (every tab's header), the tab
  bar, and the active tab's view. Disconnect lives on the connection chip,
  behind a confirm dialog.
- `src/state.rs`: the `Session` entity: client lifetime, event pump, roster
  cache (synced from the client on mesh events, when listening starts and
  every 2 minutes; presence refreshed every 30 seconds), chat history, unread
  counts, transfer state, forwards.
- `src/mesh.rs`: `prepare`/`join`/`device_npub`/`members_of`, blocking
  helpers over `noscat_client::Client`: they use the app's running client for
  the mesh directory, or run the device for the call (one device per
  directory; `Client::start` locks it). Call from the background executor
  only.
- `src/paths.rs`: XDG data locations (device dir `device/` with the
  identity, `members.json`, settings, inbox) and load/store helpers.
- `src/views/`: `join` (step 1 paste the inviting device's pairing code or
  npub, step 2 wait for approval with a countdown and this device's ID to
  compare; this device's QR sits under "Inviting from the noscat CLI
  instead?"; a device with a mesh sees Offline with Connect),
  `devices`, `chat`, `files`, `services` (expose plus port forwards and
  the inbound file log),
  `settings` (status, device name, diagnostics, Disconnect, Forget mesh).
  Forget is `Session::forget`: `mesh::leave` (30 s; with no member online
  it forgets locally and the next member online completes the removal),
  drop the client, `mesh::wipe` once `Client::running` is gone. If
  leaving fails, nothing is deleted.
- `scripts/run.sh`: runs the built binary in the dev image, for hosts
  missing a runtime library: Wayland socket, session bus, GPU, fonts, host
  networking, and the real data dir and Downloads. The default (Wayland
  only, no `x11` feature) build also runs directly on this host; the `x11`
  feature needs libxkbcommon-x11, which the host lacks.
- `src/tray.rs`: StatusNotifier tray (Linux first): show/hide window, quit.
  The node keeps running when the window closes, like Android's foreground
  service.

## Runtime behavior the app must handle (mirrors the Android phase-1 list)

- Members advertise inside the group every two minutes; a member is online
  while its latest advert is under ten minutes old. The running client
  follows the group live, so the roster comes from `Client::members` and
  updates on `Roster`/`MemberSeen` events; presence ages forward between
  them.
- Connections drop on network change (`Event::TunnelClosed`) and come back
  on the next use. `Session::spawn_network_watch` polls this machine's
  addresses every 3 s (`if-addrs`, loopback and IPv6 link-local left out)
  and calls `Client::network_changed` when they change, as the Android app
  does on a new default network; without it paths take a minute or two
  to recover. noscat also retries a failed chat send once on a fresh
  connection.
- Joining: the join screen runs `mesh::join`, which sends the named member a
  join request and waits for the invitation (accepted from that member
  only). The temporary device it runs is dropped before `start_node`, which
  frees the directory. A pairing code with `?relay=` (a private mesh) joins
  through those relays alone: `mesh::join` starts the device with them as
  `ClientConfig::join_relays`, so nothing of it (not even its key package)
  reaches the public relays. Creating a mesh needs UI that does not exist
  yet; a CLI device creates it. Inviting is approving a join request (see
  below).
- File transfers complete asynchronously: `send_file` returns the
  transfer id at once; `TransferStarted`, `TransferProgress` (both
  directions) and the outcome arrive as events carrying that id, and
  `Session` matches them by id only against this run's (`live`) entries,
  since ids restart with each client. Inbound files land in the inbox
  automatically (node mode receives them).
- A rename in Settings goes to the running client (`set_device_name`,
  blocking, so on the background executor) and members see it within
  seconds; `MemberSeen` carries each member's current name and updates
  the roster at once.
- Join requests (`Event::JoinRequest`) wait in `Session::join_requests`
  and show on the Devices page with the asker's short ID; Approve invites
  on the background executor. A desktop notification goes out for each
  new request, for chat messages not on screen in a focused window, and
  for files received while the window is unfocused (`notify-rust`, off the
  UI thread).
- Chats persist in `chats.json` (500 per conversation); Forget deletes it.
- Settings > Reachable from the internet (`Settings::reachable`) starts the
  client with `reachable: Some(None)` (public address by STUN) on the fixed
  UDP port 41641; toggling it restarts a running device in the same mode
  (`Session::set_reachable`). The app cannot check the router forward.
- Presence dots (`DeviceEntry::presence`): green = `connected` (a direct
  link or live connection, from the client's `Member.connected`; never
  saved), amber = advert fresh but no connection ("online, can't
  connect"), grey = offline. The client re-dials online members every
  30 s while running and emits `Roster` on every connection change.
- `keystore::init` runs first in `main`: a 32-byte storage key from the
  Secret Service keyring (created there on first use) goes to
  `noscat_client::set_storage_key`, sealing `identity.nsec` and
  `mesh-state.json`; plain files migrate. No keyring: plain files, as
  before. Sealed files whose key cannot be read: the key is never
  replaced, the device refuses to start, Settings says why. Settings >
  Diagnostics shows which.
- Chat is asynchronous: the optimistic bubble is marked delivered on
  `ChatDelivered` (the member acknowledged it), and failure surfaces as
  `ChatFailed`. A failed message offers Retry (the failed bubble goes, the
  text is sent as a new message) and Delete; any message can be deleted
  (hover). Nothing is resent automatically: the member may have it.
- Order: the device list keeps noscat's members() order (this device,
  then by name, then unnamed; presence never reorders). The chat list
  shows every member, latest conversation first, then device order.
- `Event::Stopped` means a background operation (listener, forward, expose)
  died; the UI must surface it and offer restart. A stopped `node` or
  `listener` drops the client and goes offline, so the connect screen's
  Connect is the restart.
- Expose replaces node mode (`expose` accepts no files); Services > Stop
  restarts in node mode (`Session::stop_expose`). Swapping roles while a
  client runs goes through `Session::restart`, which sets `restarting` so
  the shell keeps the tabs instead of flashing the connect screen.
- Files dropped on a device row (Devices) or an open conversation (Chat)
  are sent to that device (`ExternalPaths`).

## Conventions

- Single binary crate, edition 2024, MIT.
- Commit messages: imperative, no em dashes.
- Keep dependency count small and pinned by the lockfile so builds are
  reproducible from a clean checkout.
- When noscat releases, bump `noscat-client`/`noscat-core` together (they
  version-lock in the upstream workspace).
- Work on `main`; commit and push there. No tags until the 0.5.0 release.
