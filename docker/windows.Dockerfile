# Windows cross-build image: Rust 1.96.0 with x86_64-pc-windows-gnu, mingw-w64
# to link and wine (10, from trixie: Rust std needs bcryptprimitives.dll) to
# run the tests and the binary.
FROM --platform=linux/amd64 rust:1.96.0-trixie
RUN apt-get update \
 && apt-get install -y --no-install-recommends gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64 wine wine64 rsync \
 && rm -rf /var/lib/apt/lists/* \
 && rustup target add x86_64-pc-windows-gnu
# The components rust-toolchain.toml asks for, so no run downloads them.
RUN rustup component add clippy rustfmt rust-src
ENV CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=wine \
    WINEDEBUG=-all \
    WINEPREFIX=/tmp/wine
WORKDIR /work
