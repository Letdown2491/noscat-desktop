# noscat-desktop dev image: the host is atomic (bootc) and has no toolchain,
# so cargo runs in this container. Same pattern as noscat and bitchat-rs:
#   Build:  podman build -t localhost/noscat-desktop-dev -f containers/noscat-desktop-dev.containerfile containers/
#   Check:  podman run --rm -v "$PWD:/work:Z" -w /work localhost/noscat-desktop-dev cargo check
#   Test:   podman run --rm -v "$PWD:/work:Z" -w /work localhost/noscat-desktop-dev cargo test
#
# Fedora 43 base keeps the built binaries' glibc in step with the host
# (f43 carries 2.42-era glibc). gcc is needed for the C glue in ring and
# rust-secp256k1 (nostr's crypto stack). GPUI's font stack needs a C++
# compiler and the fontconfig/freetype dev packages (yeslogic-fontconfig-sys
# and freetype-sys run pkg-config in their build scripts), and the xkbcommon
# dev libs for the final link.
# noscat 0.5's FIPS data plane links libdbus (dbus-devel) and generates
# bindings with bindgen (clang-devel).
# The app can also run in this image (scripts/run.sh) on hosts missing a
# runtime library: GPUI draws with Vulkan (vulkan-loader, Mesa drivers) on
# the host's Wayland socket (libwayland-client/cursor). libxkbcommon-x11 is
# only for builds with the x11 feature.

FROM fedora:43

# Rust toolchain baked into the image: CARGO_HOME/RUSTUP_HOME live in image
# layers (not volumes), so the image is self-contained; only target/ is
# shared with the host checkout.
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:/usr/bin:/bin
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal

RUN dnf -y install gcc gcc-c++ pkgconf-pkg-config git \
    fontconfig-devel freetype-devel \
    libxkbcommon-devel libxkbcommon-x11-devel \
    dbus-devel clang-devel \
    vulkan-loader mesa-vulkan-drivers libwayland-client libwayland-cursor \
  && dnf clean all
