# Contributing

## Building

```sh
cargo build
```

The toolchain is pinned in `rust-toolchain.toml` (Rust 1.96.0). Formatting and clippy use `nightly-2026-05-22`, as in CI.

## Checks a pull request must pass

CI (`.github/workflows/ci.yml`) runs these. Run them before you push.

```sh
cargo +nightly-2026-05-22 fmt --all --check
cargo +nightly-2026-05-22 clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo-nextest nextest run --workspace --all-features --locked --profile ci
cargo test --workspace --all-features --locked --doc
cargo spellcheck check -m 1
cargo deny --config .config/deny.toml check
cargo machete
```

`cargo spellcheck` needs `libclang`. New words go into `spellcheck.dic`, one per line, and the count on its first line is updated. Nextest retries a failed test twice, because loopback connects to the fakes can stall on a busy machine; a test that passes on a retry is reported as flaky.

## Layout

```
crates/
  holst-core/    commands, domain, remote, storage, the fake server
  holst-cli/     the `holst` binary
  holst-mcp/     the `holst-mcp` server and its setup
  holst-form/    terminal prompts shared by both binaries
  holst-macros/  derive macros for commands and endpoints
tasks/
  scenarios/     end to end scenarios and the `holst-eval` harness
scripts/         release.zig
```

## Tests

No test reaches the network. `holst_core::fake` is a local server that answers the Holst API and logs every request it gets, and tests point the base URL at it. No test touches the real keychain: each one sets a temporary `HOLST_HOME` (the login is then stored there, not in the keychain) and a temporary `HOLST_CACHE_DIR`. Never print tokens, cookies or codes in a test.

## Releases

When CI passes on a push to `main`, `.github/workflows/tag.yml` reads the `holst-cli` version and creates the tag `v<version>` if it does not exist. It then starts `.github/workflows/release.yml`. That workflow checks the tag against the Cargo version, builds the five targets (Linux as static musl through `cargo-zigbuild`), writes `checksums.txt`, creates the GitHub release, publishes the crates to `crates.io` when `CARGO_REGISTRY_TOKEN` is set, and updates the `homebrew-holst` tap. To release, raise the version in `Cargo.toml`, add a section to `CHANGELOG.md`, and merge.

## Releasing without GitHub runners

The release pipeline (`.github/workflows/release.yml`) is the main path. When the runners are down, `scripts/release.zig` builds the same ten archives and `checksums.txt` on one Mac into `dist/`: `holst-cli-<target>` and `holst-mcp-<target>` for each of the five targets. The Linux targets are static musl builds made with `cargo-zigbuild` (they run on any distribution, no glibc requirement; the script checks that `file` reports both binaries as statically linked), Windows uses `cargo-xwin`.

Needs Zig 0.17.0 and cargo-zigbuild 0.24.0 exactly (the script stops on any other version), `cargo install --locked cargo-zigbuild --version 0.24.0`, `cargo install --locked cargo-xwin`, `llvm` (for `clang-cl`) on `PATH`, and the five `rustup` targets. The script prints what is missing and installs nothing. With the Homebrew package `llvm@22`, `clang-cl` is not on `PATH` by default, so run the script as `PATH=/opt/homebrew/opt/llvm@22/bin:$PATH zig run ...`.

```sh
git tag v0.3.0              # local tag; the Cargo version must match
PATH=/opt/homebrew/opt/llvm@22/bin:$PATH zig run scripts/release.zig -- v0.3.0
PATH=/opt/homebrew/opt/llvm@22/bin:$PATH zig run scripts/release.zig -- v0.3.0 --publish   # after pushing the tag; runs the same gh release create
```

Publishing to `crates.io` and updating `homebrew-holst` are not done by the script; it prints what to run by hand.
