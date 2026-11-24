# noscat desktop

Desktop client for [noscat](https://github.com/Letdown2491/noscat), the
Nostr-native mesh. Built with GPUI (via gpui-kit) in pure Rust: no FFI layer,
the protocol crate `noscat-client` is embedded directly. The UI targets
feature parity with the phase-1 Android client (chat, files, port forwards,
mesh join by invite) plus the easy desktop extras (expose/serve).

Status: early development. See [AGENTS.md](AGENTS.md) for roadmap and agent
guidance.

## Layout

- `src/state.rs` - the session: owns the running `noscat_client::Client`,
  the event pump into GPUI, the roster cache, chat history, unread counts,
  and transfer state. This is the desktop counterpart of the Android app's
  `ClientHolder`.
- `src/mesh.rs` - mesh helpers (prepare, join, members), the Rust-to-Rust
  counterpart of the upstream `noscat-ffi` crate's `mesh_*` functions.
- `src/views/` - one module per tab: Devices, Chat, Files, Shares (expose and
  inbound), Settings, and the mesh join flow.

## Data locations

Everything lives under the XDG data dir (`~/.local/share/noscat-desktop` on
Linux):

- `device/` - this app's device: `identity.nsec` (its own noscat identity,
  mode 0600, generated on first run, separate from the CLI's so the desktop
  app is its own device) and the mesh state. Membership is what lets other
  devices reach it.
- `members.json` - the cached member list so startup renders instantly.
- 0.4's `mesh/` and `roster.json` are not read by 0.5 (a fresh start: join
  the new mesh again) and can be deleted.
- `settings.json` - device name and options.
- Inbound files land in `~/Downloads/noscat` (or `inbox/` under the data dir
  when Downloads is unavailable).

## Building

Plain cargo works wherever a Rust toolchain exists:

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

On immutable hosts the GPUI runtime libraries are not installed either, so the
app runs in the same image:

```
scripts/run.sh            # target/release; `scripts/run.sh debug` for a debug build
```

It passes in the Wayland socket, session bus, GPU and fonts, uses host
networking, and keeps app data in `~/.local/share/noscat-desktop` and received
files in `~/Downloads/noscat` as a native install would. Wayland only; Ctrl+C
quits.

## License

MIT
