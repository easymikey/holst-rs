# Contributing

## Releasing without GitHub runners

The release pipeline (`.github/workflows/release.yml`) is the main path. When the runners are down, `scripts/release.zig` builds the same five archives and `checksums.txt` on one Mac into `dist/`. Linux targets use `cargo-zigbuild` with `glibc` 2.35 (the ubuntu-22.04 floor), Windows uses `cargo-xwin`.

Needs Zig 0.17.0 exactly (the script stops on any other version), `cargo install --locked cargo-zigbuild cargo-xwin`, `llvm` (for `clang-cl`) on `PATH`, and the five `rustup` targets. The script prints what is missing and installs nothing.

```sh
git tag v0.3.0              # local tag; the Cargo version must match
zig run scripts/release.zig -- v0.3.0
zig run scripts/release.zig -- v0.3.0 --publish   # after pushing the tag; runs the same gh release create
```

Publishing to `crates.io` and updating `homebrew-holst` are not done by the script; it prints what to run by hand.
