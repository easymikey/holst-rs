# Contributing

## Building

```sh
cargo build
```

The toolchain is pinned in `rust-toolchain.toml` (Rust 1.99.1). Clippy runs on that stable toolchain; the nightly named in `justfile` is used only for formatting.

## Tools and checks

Every tool is pinned in `[workspace.metadata.bin]` of `Cargo.toml` and run through [cargo-run-bin](https://github.com/dustinblackman/cargo-run-bin), so you and CI use the same versions. Install it once; each tool is built into `.bin/` on first use.

```sh
cargo install --locked cargo-run-bin
rustup toolchain install nightly-2026-10-30 -c rustfmt --profile minimal
```

The commands live in `justfile`, one recipe per CI job (`.github/workflows/ci.yml` runs the same recipes):

```sh
cargo bin just fmt      # format the code
cargo bin just lint     # formatting and spelling
cargo bin just deps     # licences and advisories, unused dependencies, each feature on its own
cargo bin just check    # clippy, tests and doctests
cargo bin just ci       # all of the above: run it before you push
```

`cargo bin just` lists the recipes. The recipes call each tool as `cargo bin cargo-<tool>`, not through cargo aliases, which would shadow a copy installed globally.

`cargo spellcheck` needs `libclang` to build: on macOS `brew install llvm` and set `LIBCLANG_PATH=$(brew --prefix llvm)/lib` the first time. New words go into `spellcheck.dic`, one per line, and the count on its first line is updated. Nextest retries a failed test twice, because loopback connects to the fakes can stall on a busy machine; a test that passes on a retry is reported as flaky.

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

Needs Zig 0.17.0 exactly (the script stops on any other version), `cargo-run-bin` (it runs the `cargo-zigbuild` and `cargo-xwin` pinned in `Cargo.toml`), `llvm` (for `clang-cl`) on `PATH`, and the five `rustup` targets. The script prints what is missing and installs nothing. With the Homebrew package `llvm@22`, `clang-cl` is not on `PATH` by default, so run the script as `PATH=/opt/homebrew/opt/llvm@22/bin:$PATH zig run ...`.

```sh
git tag v0.3.0              # local tag; the Cargo version must match
PATH=/opt/homebrew/opt/llvm@22/bin:$PATH zig run scripts/release.zig -- v0.3.0
PATH=/opt/homebrew/opt/llvm@22/bin:$PATH zig run scripts/release.zig -- v0.3.0 --publish   # after pushing the tag; runs the same gh release create
```

Publishing to `crates.io` and updating `homebrew-holst` are not done by the script; it prints what to run by hand.
