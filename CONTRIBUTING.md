# Contributing

## Setup

```sh
cargo install cargo-run-bin --locked
rustup toolchain install
rustup toolchain install "$(cargo bin just --evaluate nightly)" --profile minimal --component rustfmt
cargo bin just hooks
```

- `rust-toolchain.toml` pins the stable toolchain, and `rustup toolchain install` installs it. The nightly named in `justfile` is used only for formatting.
- Every other tool, `just` included, is pinned in `Cargo.toml` and runs as `cargo bin <tool>`. It builds into `.bin/` on first use, or comes prebuilt when [`cargo-binstall`](https://github.com/cargo-bins/cargo-binstall) is installed.
- `cargo bin just hooks` points git at `.githooks/`: `pre-commit` runs `fmt-check` and `spell` (seconds), `commit-msg` checks the subject, `pre-push` runs `clippy`.

## Checks

- `cargo bin just` runs `default`, which runs every group. Run it before a pull request.
- Each recipe runs one tool, and a group only lists recipes. `cargo bin just --list` shows both; `cargo bin just clippy` or `cargo bin just lint` runs one.
- `cargo bin just fmt` formats the code.
- Recipes call each tool as `cargo bin cargo-<tool>`, not through a cargo alias, which would shadow a copy installed globally.
- CI (`.github/workflows/ci.yml`) runs the same recipes after the shared setup action `.github/actions/setup`. Each runner in its matrix names its recipes. On Linux CI also checks every commit subject in the push or pull request with `cargo bin just commits <range>`.
- The spell check is [`typos`](https://github.com/crate-ci/typos). A real word it flags goes under `[default.extend-words]` in `_typos.toml`.

## Commit messages

[Conventional Commits](https://www.conventionalcommits.org/): `type(scope): subject`, the subject in lower case and without a version number. The `commit-msg` hook refuses anything else, and `cargo bin just commits` checks a range (`origin/main..HEAD` by default). Scopes are listed in the project section below.

The type does not decide the version or the changelog: the maintainer picks the level at release and the changelog is written by hand. A breaking change takes `!` after the type (`feat!: ...`).

## Releases

- Each crate has its own version and a tag `<crate>-v<version>`. One `CHANGELOG.md` at the root serves every crate.
- Write the changelog by hand under `## Unreleased`, in the same commit as the change: [Keep a Changelog](https://keepachangelog.com) groups `Added`, `Changed`, `Fixed` and `Removed`, one line per change that starts with what the user types or sees, no scopes. A breaking change goes in a first paragraph of the version, `Breaking: ...`, that says what to do.
- `cargo bin just ship <level>` (`patch`, `minor` or `major`) runs every check, the semver check and a dry run of `cargo-release`. It needs a clean `main`. Add `execute` (`cargo bin just ship patch execute`) to release for real: it bumps the versions, renames `## Unreleased` to `## <version> (<date>)`, commits `chore: release`, tags, publishes and pushes. Run `cargo login` once on the maintainer's machine first.
- A tag push runs the `release` workflow, which makes a draft GitHub release from the changelog section with `cargo bin just release-notes <tag>`. For `holst-cli` and `holst-mcp` it then builds and attaches the binaries and updates Homebrew (see below). It publishes the draft last. It fails when the section is missing or empty, and it runs only in the `easymikey` repository. CI holds no crates.io token.

## Layout

```
crates/
  holst-core/    commands, domain, remote, storage, the fake server
  holst-cli/     the `holst` binary
  holst-mcp/     the `holst-mcp` server and its setup
  holst-macros/  derive macros for commands and endpoints
tasks/
  scenarios/     end to end scenarios and the `holst-eval` harness
```

## Holst

- Scopes: a crate name without `holst-` (`cli`, `mcp`, `core`, `form`) or a module (`workspace`, `storage`, `error`).
- Groups: `lint` (`fmt-check`, `spell`, `clippy`), `check` (`test`, `doctest`, `doc`), `deps` (`deny`, `machete`), plus the recipes `features` and `budget`.
- `features` checks each feature on its own with `cargo-hack`.
- `budget` builds the release binaries and checks the size and median startup of `holst` and `holst-mcp` against the limits at the top of `justfile`. The limits are for an Apple silicon Mac, so CI skips it. It is slow.
- CI runs `lint check deps features` on Linux x64, and `clippy check` on Linux arm, macOS and Windows. With `full: true` the setup action also installs Zig.
- `clippy.toml` denies direct file I/O, environment reads, stdout and stderr, clock calls and process spawning outside the modules that own them. Each lint message names the module to go through.
- No test reaches the network: `holst_core::fake` is a local server that answers the Holst API and logs every request, and tests point the base URL at it.
- No test touches the keychain: each sets a temporary `HOLST_HOME` (the login is stored there) and a temporary `HOLST_CACHE_DIR`.
- Never print tokens, cookies or codes in a test.
- Nextest retries a failed test twice, because loopback connects to the fakes can stall on a busy machine. A test that passes on a retry is reported as flaky.
- Tags are `holst-cli-vX.Y.Z`, `holst-mcp-vX.Y.Z`, `holst-core-vX.Y.Z` and `holst-macros-vX.Y.Z`. `cargo-release` applies one level to the whole workspace, so `ship` bumps all four together; the root changelog heading is renamed by the `[package.metadata.release]` of `holst-cli`, the one crate every release includes.
- `ship` runs `cargo-semver-checks` (pinned like the other tools) before the release step; an API break it finds needs a higher level.
- `holst-scenarios` is never released. `holst-form` (replaced by clacky) is no longer in the tree; its last version 0.4.1 is never yanked.
- After a `holst-cli` or `holst-mcp` tag is pushed, the `release` workflow builds its archives for five targets with `cargo bin just dist <target>`, attaches them with their `.sha256` files to the draft release with `cargo bin just upload <tag> <target>`, updates the `homebrew-holst` tap with `cargo bin just homebrew <tag>`, then publishes the draft. The tap push needs the `TAP_DEPLOY_KEY` secret in CI, or SSH push access to `easymikey/homebrew-holst` locally.
- The Linux targets are static musl builds made with `cargo-zigbuild`. The recipe fails unless `file` reports both binaries as statically linked.
- Without GitHub runners, run by hand: `cargo bin just release-notes <tag> > notes.md`, `gh release create <tag> --verify-tag --draft --title <tag> --notes-file notes.md`, `cargo bin just dist <target>` and `cargo bin just upload <tag> <target>` for each target, `cargo bin just homebrew <tag>`, then `gh release edit <tag> --draft=false`.
- The Windows target cross-builds with `cargo-xwin`, which needs `clang-cl` on `PATH`. With the Homebrew `llvm@22` that is `PATH=/opt/homebrew/opt/llvm@22/bin:$PATH`.
