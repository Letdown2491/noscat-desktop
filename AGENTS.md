# AGENTS.md

Guidance for AI agents working in this repo. Read this file fully before
making changes.

## What this project is

The desktop client for [noscat](https://github.com/Letdown2491/noscat), a
Nostr-native mesh VPN (Noise XK over UDP, Nostr as the control plane).
Identity is an npub. This repo is pure Rust on GPUI (via gpui-kit): it embeds
the published `noscat-client` crate directly, with no FFI layer. The protocol
lives in the noscat workspace (`noscat-core`/`noscat-client`), never in this
repo.

**Feature contract:** parity with the phase-1 Android client (see the
noscat-android AGENTS.md), plus the expose/serve extras that are cheap on
desktop. Phase 1 scope: mesh join by invite, Devices roster with presence,
Chat (conversations, optimistic bubbles, delivered/failed), Files (inbox and
live transfer progress), port forwards, Settings (status, device name,
diagnostics), disconnect behind a confirm dialog. Desktop extras: expose a
local TCP service, system tray so the node keeps running when the window
closes.

## Non-negotiable rules

0. **Write only in this repo.** The `noscat` (crates/CLI) and
   `noscat-android` repos may be read for reference, never modified
   directly, even when a bug lives there: report it as a GitHub issue on
   `Letdown2491/noscat` (or `noscat-android`) and let that project fix it.
   This rule outranks any user request to "just patch it" upstream.
1. **Zero protocol code in this repo.** If a change needs Noise, UDP framing,
   Nostr events, relay logic, advert handling, or new client surface, it
   belongs in the noscat repo's `noscat-client` crate, published to
   crates.io, then bumped here. The one deliberate exception is `src/mesh.rs`:
   the mesh helpers are app-layer composition over `noscat-core` (the same
   code the FFI crate carries), not protocol code.
2. **The UI sees only safe surfaces: the user's language, not the wire's.**
   Plain commands (`forward`, `send_file`, `send_chat`, `up`/`expose`), the
   mesh helpers, and the event stream reach view code. Never leak
   ports-as-socket-addrs, hex keys, or identity strings (npubs) into what
   the UI renders: identity is shown as a scannable QR or a copy action
   ("device ID"), device names and human presence text stand in for
   crypto material, and short-npub display helpers are banned.
3. **No em dashes anywhere in the repo** (use plain hyphens). Applies to code
   comments, docs, commit messages, everything.
4. **Events arrive on a background thread.** The event pump thread forwards
   `noscat_client::Event` values into the GPUI executor; state mutation only
   happens inside `Session::update` on the main thread. Views never touch the
   client directly, only the `Session` entity.
5. **No blocking client calls on the main thread.** `Client::connect`, the
   mesh helpers, and `listen`-family calls block for seconds (relay fetches,
   hole punching). They run on the background executor; results come back as
   state updates.
6. **Client state survives view churn.** The roster persists to
   `roster.json` so startup renders the device list instantly; presence
   entries age forward instead of flashing live peers offline when a relay
   fetch comes back empty.
7. **GitHub pushes go over SSH** (`git@github.com:...`), not HTTPS.

## Layout

- `src/main.rs` - GPUI bootstrap, window options, tray lifecycle hookup.
- `src/app.rs` - the root view: connection chip (every tab's header), the tab
  bar, and the active tab's view. Disconnect lives on the connection chip,
  behind a confirm dialog.
- `src/state.rs` - the `Session` entity: client lifetime, event pump, roster
  cache with a 2 minute sync timer plus a sync when listening starts, chat
  history, unread counts, transfer state, forwards.
- `src/mesh.rs` - `mesh_prepare`/`mesh_join`/`mesh_sync`/`mesh_members`,
  blocking helpers over `noscat_core::mesh::Device`; call from the background
  executor only.
- `src/paths.rs` - XDG data locations (identity, mesh dir, roster, settings,
  inbox) and load/store helpers.
- `src/views/` - `join` (paste invite or owner npub, display own npub and QR),
  `devices`, `chat`, `files`, `shares` (expose plus inbound file log),
  `settings` (status, device name, diagnostics, disconnect).
- `src/tray.rs` - StatusNotifier tray (Linux first): show/hide window, quit.
  The node keeps running when the window closes, like Android's foreground
  service.

## Runtime behavior the app must handle (mirrors the Android phase-1 list)

- Adverts expire; a listening client must re-advertise (the client runtime
  does this) and tunnels drop on network change (`Event::TunnelClosed`) then
  re-establish.
- The mesh sync enriches membership with advert names and presence
  (`last_seen_secs`, green/red dot). A per-device advert fetch can come back
  empty (relays are flaky), so a sync keeps the previous name and presence,
  aged forward, instead of flashing a live peer red.
- File transfers complete asynchronously: `send_file` returns immediately;
  progress, success, and failure arrive as `TransferProgress` /
  `TransferDone` / `TransferFailed` events. Inbound files land in the inbox
  automatically (node mode receives them).
- Chat is asynchronous: the optimistic bubble is marked delivered when the
  peer echoes the delivery FIN, and failure surfaces as `ChatFailed`.
  Upstream note: `Client::send_chat` currently emits only failures; a
  delivered event may need to be added upstream. Until then the UI shows
  "sent" on the optimistic bubble.
- `Event::Stopped` means a background operation (listener, forward, expose)
  died; the UI must surface it and offer restart.

## Conventions

- Single binary crate, edition 2024, MIT.
- Commit messages: imperative, no em dashes.
- Keep dependency count small and pinned by the lockfile so builds are
  reproducible from a clean checkout.
- When noscat releases, bump `noscat-client`/`noscat-core` together (they
  version-lock in the upstream workspace).
