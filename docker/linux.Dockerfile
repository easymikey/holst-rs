# Linux check image: Rust 1.96.0, node + python3 for the CLI end-to-end check,
# dbus + gnome-keyring for the Secret Service runs.
FROM rust:1.96.0-bookworm
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      nodejs python3 dbus dbus-x11 gnome-keyring libsecret-tools rsync \
 && rm -rf /var/lib/apt/lists/*
# The components rust-toolchain.toml asks for, so no run downloads them.
RUN rustup component add clippy rustfmt rust-src
WORKDIR /work
