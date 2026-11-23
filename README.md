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
- `src/mesh.rs` - mesh helpers (prepare, join, sync, members), a Rust-to-Rust
  copy of the glue in the upstream `noscat-ffi` crate.
- `src/views/` - one module per tab: Devices, Chat, Files, Shares (expose and
  inbound), Settings, and the mesh join flow.

## Data locations

Everything lives under the XDG data dir (`~/.local/share/noscat-desktop` on
Linux):

- `identity.hex` - this app's own noscat identity (mode 0600, generated on
  first run). Deliberately separate from the CLI's `~/.noscat` identity so the
  desktop app is its own device in the mesh.
- `mesh/` - the mesh device directory (MLS state), the thing that authorizes
  mesh-scoped tunnels.
- `roster.json` - the persisted device list so startup renders instantly.
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
podman run --rm -v noscat-desktop-cargo-home:/usr/local/cargo:Z -v "$PWD:/work:Z" \
  -w /work localhost/noscat-desktop-dev sh -c "cargo check"
```

Note the container build has no display, so `cargo check` and `cargo test` are
the container workflow; `cargo run` happens on the host or in any toolchain
bearing environment with a Wayland/X11 session.

## License

MIT
