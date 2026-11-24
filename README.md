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
Cargo.toml) and cannot talk to 0.4 devices. See [AGENTS.md](AGENTS.md) for
agent guidance.

## Joining a mesh

The app cannot create a mesh or invite devices yet; a device running the
noscat CLI does that.

1. On a computer in the mesh, run `noscat up` (with `--accept-joins` while
   you add devices, if you want joins approved automatically).
2. Copy that device's ID (`noscat id` prints it) and paste it into the
   app's join screen, then press **Request invite and join**. The app sends
   that device a join request and waits up to two minutes.
3. On the CLI, type the `invite npub1...` line it prints (not needed with
   `--accept-joins`). The app connects by itself.

Every mesh needs one member reachable from the internet; on one LAN, devices
find each other directly.

## Disconnect and Forget

Settings has both. **Disconnect** takes this computer offline but keeps it in
the mesh; connect again from the start screen. **Forget mesh** asks the other
members to remove it, then deletes its identity and mesh data here (received
files stay); to come back, join again as a new device. If no member is online,
the removal completes when one next comes online.

## Layout

- `src/state.rs` - the session: owns the running `noscat_client::Client`,
  the event pump into GPUI, the roster cache, chat history, unread counts,
  and transfer state. This is the desktop counterpart of the Android app's
  `ClientHolder`.
- `src/mesh.rs` - mesh helpers (prepare, join, members, leave, wipe), the Rust-to-Rust
  counterpart of the upstream `noscat-ffi` crate's `mesh_*` functions.
- `src/views/` - one module per tab: Devices, Chat, Files, Services (expose,
  port forwards and inbound files), Settings, and the mesh join flow.
- `scripts/run.sh` - runs the built app in the dev container, for hosts
  missing a runtime library (see Running).

## Data locations

Everything lives under the XDG data dir (`~/.local/share/noscat-desktop` on
Linux):

- `device/` - this app's device: `identity.nsec` (its own noscat identity,
  mode 0600, generated on first run, separate from the CLI's so the desktop
  app is its own device), the mesh state (`mesh-state.json`,
  `relay-sync.json`) and a `lock` file (one running device per directory).
  Membership is what lets other devices reach it.
- `members.json` - the cached member list so startup renders instantly.
- 0.4's `mesh/` and `roster.json` are not read by 0.5 (a fresh start: join
  the new mesh again) and can be deleted.
- `settings.json` - device name and options.
- `transfers.json` - the Files history: the last 100 sends, and who sent
  each received file. Forget mesh deletes it.
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
libxkbcommon-x11 at run time.

If the host lacks a runtime library, `scripts/run.sh` runs the app inside the
dev image instead (`scripts/run.sh debug` for a debug build). It passes in the
Wayland socket, session bus, GPU and fonts, uses host networking, and keeps app
data in `~/.local/share/noscat-desktop` and received files in
`~/Downloads/noscat` as a native install would. Ctrl+C quits.

## License

MIT
