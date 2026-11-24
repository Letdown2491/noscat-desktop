#!/bin/sh
# Runs the built app inside the dev image, for hosts missing a runtime
# library (the default Wayland-only build usually runs directly from
# target/). Build first (release by default):
#   podman run --rm -v noscat-desktop-cargo-home:/usr/local/cargo:z -v "$PWD:/work:Z" \
#     -w /work localhost/noscat-desktop-dev cargo build --release
# Usage: scripts/run.sh [debug]
#
# The container gets the host's Wayland socket, session bus (tray, file
# picker portal), GPU, fonts and host networking (UDP and LAN discovery),
# and writes app data to the host's ~/.local/share/noscat-desktop and
# received files to ~/Downloads/noscat, as a native install would.
set -eu
cd "$(dirname "$0")/.."

PROFILE="${1:-release}"
BIN="$PWD/target/$PROFILE/noscat-desktop"
[ -x "$BIN" ] || { echo "no binary at $BIN; build it first" >&2; exit 1; }
: "${WAYLAND_DISPLAY:?no Wayland session}"
RUN="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
DATA="$HOME/.local/share/noscat-desktop"
DL="$HOME/Downloads"
CACHE="$HOME/.cache/noscat-desktop"
mkdir -p "$DATA" "$DL" "$CACHE"

FONTS=""
[ -d "$HOME/.local/share/fonts" ] && FONTS="-v $HOME/.local/share/fonts:$HOME/.local/share/fonts:ro"

# label=disable: SELinux otherwise blocks the host's Wayland and bus sockets.
# keep-id: files written to the data dir and Downloads stay owned by you.
# --init: the app is not PID 1, so Ctrl+C and stop signals end it.
exec podman run --rm --init --name noscat-desktop \
  --userns=keep-id --security-opt label=disable \
  --network host --device /dev/dri \
  -e HOME="$HOME" -e XDG_RUNTIME_DIR="$RUN" -e XDG_CACHE_HOME="$CACHE" \
  -e WAYLAND_DISPLAY="$WAYLAND_DISPLAY" \
  -e DBUS_SESSION_BUS_ADDRESS="unix:path=$RUN/bus" \
  -e RUST_LOG="${RUST_LOG:-warn}" \
  -v "$RUN/$WAYLAND_DISPLAY:$RUN/$WAYLAND_DISPLAY" \
  -v "$RUN/bus:$RUN/bus" \
  -v /usr/share/fonts:/usr/share/fonts:ro $FONTS \
  -v "$DATA:$DATA" -v "$DL:$DL" -v "$CACHE:$CACHE" \
  -v "$BIN:/usr/local/bin/noscat-desktop:ro" \
  localhost/noscat-desktop-dev /usr/local/bin/noscat-desktop
