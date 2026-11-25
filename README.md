# noscat desktop

Desktop client for [noscat](https://github.com/Letdown2491/noscat), a
private device mesh on Nostr: membership is a Marmot (MLS) group on Nostr
relays, and data goes device to device over FIPS. Built with GPUI (via
gpui-kit) in pure Rust: no FFI layer, the protocol crate `noscat-client` is
embedded directly. The UI targets feature parity with the Android client
(chat, files, port forwards, joining a mesh) plus the easy desktop extras
(expose/serve, a tray icon that keeps the device running).

Status: early development, on noscat 0.5.0, which is not released yet: the
app builds against noscat's `main` branch (`[patch.crates-io]` in
Cargo.toml). See [AGENTS.md](AGENTS.md) for
agent guidance.

## Joining a mesh

The app cannot create a mesh yet; a device running the noscat CLI does
that (`noscat mesh create <name>`).

1. On a device in the mesh, show its pairing code: `noscat mesh qr` on a
   CLI device, or Settings > Device ID in the apps.
2. Paste that code (or the bare `npub1...`) into the app's join screen
   and press **Request invite and join**. The app sends that device a join
   request and waits up to two minutes.
3. Approve the request on that device: Approve on the apps' Devices tab;
   on a CLI running `noscat up` in a terminal, type the `invite npub1...`
   line it prints; on a CLI running as a service, run
   `noscat mesh requests` and answer y. The app connects by itself.

Once this computer is in the mesh, it approves join requests too: a device
that pastes or scans this computer's code shows up on the Devices tab with
Approve and Decline. Check the short device ID against the joining
device's screen before approving.

A private mesh (`noscat mesh private`, on a members-only relay such as
noscat-relay) is joined with the member's whole pairing code instead
(`noscat mesh qr` prints it: `noscat:npub1...?relay=...`); the app then
joins through that relay alone, and publishes nothing to public relays. In a private mesh, the QR under Settings >
Device ID names the mesh's relays too.

For devices on different networks to connect, one member must be reachable
from the internet (Settings > Reachable from the internet, or a server running
`noscat up --reachable`); on one LAN, devices find each other directly.

## Devices and chat

Devices are listed this computer first, then by name; being online never
moves a device. The Chat tab lists every member, latest conversation first.
A message the other device never acknowledged is marked "not delivered"
with **Retry** (sends the text again as a new message) and **Delete**;
hovering over any message shows Delete. Nothing is resent automatically,
since the other device may have received it. When this computer's network
changes (Wi-Fi to Ethernet, a hotspot, a VPN), the app notices within a few
seconds and tells noscat, so connections recover quickly.

## Presence

Each device's dot says whether data can reach it: **green**, connected now;
**amber**, it is online (its announcements arrive) but nothing can connect to
it, usually because it is on another network and no member is reachable;
**grey**, offline.

## Disconnect and Forget

Settings has both. **Disconnect** takes this computer offline but keeps it in
the mesh; connect again from the start screen. **Forget mesh** asks the other
members to remove it, then deletes its identity and mesh data here (received
files stay); to come back, join again as a new device. If no member is online,
the removal completes when one next comes online.

## Layout

- `src/state.rs`: the session: owns the running `noscat_client::Client`,
  the event pump into GPUI, the roster cache, chat history, unread counts,
  and transfer state. This is the desktop counterpart of the Android app's
  `ClientHolder`.
- `src/mesh.rs`: mesh helpers (prepare, join, members, leave, wipe), the Rust-to-Rust
  counterpart of the upstream `noscat-ffi` crate's `mesh_*` functions.
- `src/views/`: one module per tab: Devices, Chat, Files, Services (expose,
  port forwards and inbound files), Settings, and the mesh join flow.
- `scripts/run.sh`: runs the built app in the dev container, for hosts
  missing a runtime library (see Running).

## Data locations

Everything lives under the XDG data dir (`~/.local/share/noscat-desktop` on
Linux):

- `device/`: this app's device: `identity.nsec` (its own noscat identity,
  mode 0600, generated on first run, separate from the CLI's so the desktop
  app is its own device), the mesh state (`mesh-state.json`,
  `relay-sync.json`) and a `lock` file (one running device per directory).
  Membership is what lets other devices reach it.
- `members.json`: the cached member list so startup renders instantly.
- `settings.json`: device name and options, including **Reachable from
  the internet** (Settings): this computer becomes the mesh's meeting point
  for devices on other networks, like `noscat up --reachable`. It binds UDP
  41641, which the router must forward to it; one reachable device per
  mesh is enough.
- `device/identity.nsec` and `device/mesh-state.json` are encrypted with a
  key kept in the desktop keyring (GNOME Keyring, KWallet, KeePassXC);
  without a keyring they stay plain, and Settings says which.
- `chats.json`: chat history, the newest 500 messages per conversation.
  Forget mesh deletes it.
- `transfers.json`: the Files history: the last 100 sends, and the sender
  of each of the last 500 received files. Forget mesh deletes it.
- Inbound files land in `~/Downloads/noscat` (or `inbox/` under the data dir
  when Downloads is unavailable).

## Building

Plain cargo works wherever a Rust toolchain exists (on glibc Linux with the
libdbus and libclang development packages installed, which noscat's FIPS
data plane needs):

```
cargo build --release
```

On atomic (bootc) hosts there is no toolchain; build in the container like the
other noscat projects:

```
podman build -t localhost/noscat-desktop-dev -f containers/noscat-desktop-dev.containerfile containers/
podman run --rm -v noscat-desktop-cargo-home:/usr/local/cargo:z -v "$PWD:/work:Z" \
  -w /work localhost/noscat-desktop-dev sh -c "cargo check"
```

Build the binary the same way with `sh -c "cargo build --release"`.

## Running

The default build is Wayland only and needs nothing beyond a normal desktop
(libxkbcommon, libxcb, Vulkan), so it runs straight from the build directory:

```
target/release/noscat-desktop
```

Xorg sessions need `cargo build --release --features x11`, which also needs
libxkbcommon-x11 at run time (a Wayland-only machine without it cannot start
that build). Releases ship both: `noscat-desktop` (Wayland) and
`noscat-desktop-x11`.

If the host lacks a runtime library, `scripts/run.sh` runs the app inside the
dev image instead (`scripts/run.sh debug` for a debug build). It passes in the
Wayland socket, session bus, GPU and fonts, uses host networking, and keeps app
data in `~/.local/share/noscat-desktop` and received files in
`~/Downloads/noscat` as a native install would. Ctrl+C quits.

## License

MIT
