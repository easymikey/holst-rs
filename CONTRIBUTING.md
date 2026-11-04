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
- `cargo bin just hooks` points git at `.githooks/`: `pre-commit` runs `fmt-check` and `clippy`, `commit-msg` checks the subject.
- The spell check builds `cargo-spellcheck` against libclang. On Linux install `libclang-dev`. On macOS the Xcode command line tools have it; build it once with `RUSTFLAGS="-C link-arg=-Wl,-rpath,/Library/Developer/CommandLineTools/usr/lib" cargo bin cargo-spellcheck --version`.

## Checks

- `cargo bin just` runs `default`, which runs every group. Run it before a pull request.
- Each recipe runs one tool, and a group only lists recipes. `cargo bin just --list` shows both; `cargo bin just clippy` or `cargo bin just lint` runs one.
- `cargo bin just fmt` formats the code.
- Recipes call each tool as `cargo bin cargo-<tool>`, not through a cargo alias, which would shadow a copy installed globally.
- CI (`.github/workflows/ci.yml`) runs the same recipes after the shared setup action `.github/actions/setup`. Each runner in its matrix names its recipes. On Linux CI also checks every commit subject in the push or pull request with `cargo bin just commits <range>`.
- A real word the spell check flags goes into `spellcheck.dic`, one per line, and the count on its first line goes up by one.

## Commit messages

[Conventional Commits](https://www.conventionalcommits.org/): `type(scope): subject`, the subject in lower case and without a version number. The `commit-msg` hook refuses anything else, and `cargo bin just commits` checks a range (`origin/main..HEAD` by default). Scopes are listed in the project section below.

- `feat` goes into the changelog under "Added".
- `fix` goes under "Fixed".
- `refactor`, `perf` and `revert` go under "Changed".
- `docs`, `test`, `build`, `ci`, `chore` and `style` stay out of the changelog.
- A breaking change takes `!` after the type (`feat!: ...`) or a `BREAKING CHANGE:` footer, and always reaches the changelog.
- While a crate is at 0.x, `feat` and `fix` bump the patch version and a breaking change bumps the minor version.

## Releases

- Each crate has its own version, a `CHANGELOG.md` in its directory, a tag `<crate>-v<version>` and a GitHub release. [release-plz](https://release-plz.dev) (`release-plz.toml`) picks each changed crate's bump from the commits.
- A push to `main` runs CI and publishes nothing, unless its last commit is `chore: release` and CI passes.
- To release from GitHub, run the `release` workflow on `main` from the Actions tab. It runs `cargo bin just release`, commits `chore: release`, pushes it and runs `cargo bin just publish`.
- To release locally, run `cargo bin just release`, review the diff, commit it as `chore: release` and push. Once CI on `main` passes, the `release` workflow runs `cargo bin just publish`.
- `cargo bin just publish` needs `GITHUB_TOKEN` and `CARGO_REGISTRY_TOKEN`. It publishes each crate version crates.io lacks, tags it and creates its GitHub release; with nothing new it does nothing.
- The `release` workflow runs only in the `easymikey` repository, so a fork never publishes.

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
```

## Holst

- Scopes: a crate name without `holst-` (`cli`, `mcp`, `core`, `form`) or a module (`workspace`, `storage`, `error`).
- Groups: `lint` (`fmt-check`, `spell`, `clippy`), `check` (`test`, `doctest`, `doc`), `deps` (`deny`, `machete`), plus the recipes `features` and `budget`.
- `features` checks each feature on its own with `cargo-hack`.
- `budget` builds the release binaries and checks the size and median startup of `holst` and `holst-mcp` against the limits at the top of `justfile`. The limits are for an Apple silicon Mac, so CI skips it. It is slow.
- CI runs `lint check deps features` on Linux x64, and `clippy check` on Linux arm, macOS and Windows. With `full: true` the setup action also installs libclang and Zig.
- `clippy.toml` denies direct file I/O, environment reads, stdout and stderr, clock calls and process spawning outside the modules that own them. Each lint message names the module to go through.
- No test reaches the network: `holst_core::fake` is a local server that answers the Holst API and logs every request, and tests point the base URL at it.
- No test touches the keychain: each sets a temporary `HOLST_HOME` (the login is stored there) and a temporary `HOLST_CACHE_DIR`.
- Never print tokens, cookies or codes in a test.
- Nextest retries a failed test twice, because loopback connects to the fakes can stall on a busy machine. A test that passes on a retry is reported as flaky.
- release-plz runs without semver checks, so the bump comes from the commits alone. `holst-scenarios` is never released.
- The root `CHANGELOG.md` is the history up to 0.4.0 and stays as it is; the per-crate changelogs continue it.
- After `holst-cli` or `holst-mcp` is released, the `release` workflow builds its archives for five targets with `cargo bin just dist <target>`, attaches them with their `.sha256` files to that crate's GitHub release, then updates the `homebrew-holst` tap with `cargo bin just homebrew <tag>`. The tap push needs the `TAP_DEPLOY_KEY` secret in CI, or SSH push access to `easymikey/homebrew-holst` locally.
- The Linux targets are static musl builds made with `cargo-zigbuild`. The recipe fails unless `file` reports both binaries as statically linked.
- Without GitHub runners, run by hand: `cargo bin just release`, `cargo bin just publish`, `cargo bin just dist <target>` for each target, `gh release upload <tag> dist/<crate>-<target>.* --clobber`, then `cargo bin just homebrew <tag>`.
- The Windows target cross-builds with `cargo-xwin`, which needs `clang-cl` on `PATH`. With the Homebrew `llvm@22` that is `PATH=/opt/homebrew/opt/llvm@22/bin:$PATH`.
