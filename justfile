nightly := "nightly-2026-05-22"

[private]
default:
    @{{ just_executable() }} --list

# Format the code.
fmt:
    cargo +{{ nightly }} fmt --all

# CI job `lint`: formatting and spelling.
lint:
    cargo +{{ nightly }} fmt --all --check
    cargo bin cargo-spellcheck check -m 1

# CI job `check`: clippy, tests and doctests.
check:
    cargo +{{ nightly }} clippy --workspace --all-targets --all-features --locked -- -D warnings
    cargo bin cargo-nextest run --workspace --all-features --locked --profile ci
    cargo test --workspace --all-features --locked --doc

# CI job `deps`: licences and advisories, unused dependencies, each feature on its own.
deps:
    cargo bin cargo-deny --config .config/deny.toml check
    cargo bin cargo-machete
    cargo bin cargo-hack check --workspace --each-feature --locked

# Every CI job.
ci: lint deps check
