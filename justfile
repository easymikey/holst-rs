set dotenv-load

# nightly for fmt, clippy and rustdoc; build and tests follow rust-toolchain.toml
nightly := "nightly-2026-05-22"

# fmt, clippy and tests
default: fmt clippy test

# ========== local development

# tests, the filter is optional
test *FILTER:
    cargo nextest run --workspace {{FILTER}} -- ${JUST_TEST_ARGS:-}
    cargo test --workspace --doc

# fmt, spellcheck and clippy
check: fmt spellcheck clippy

# format
fmt:
    cargo +{{nightly}} fmt

# spellcheck of docs and comments (cargo-spellcheck)
spellcheck:
    cargo spellcheck check -m 1

# clippy for code and tests
clippy:
    cargo +{{nightly}} clippy --workspace --all-targets --all-features

# remove unused dependencies
shear:
    cargo shear --fix

# divan benchmarks
bench *FILTER:
    cargo bench -p holst-core {{FILTER}}


# build holst and holst-mcp from this checkout into ~/.cargo/bin
install:
    cargo install --path crates/holst-cli --locked
    cargo install --path crates/holst-mcp --locked

# build the tools from [workspace.metadata.bin]
tools:
    cargo bin --install
    cargo bin --sync-aliases

# build cargo-spellcheck (not in [workspace.metadata.bin]: it is not run through cargo bin). Its hunspell-sys
# build script links libclang; Linux needs libclang-dev, and on macOS the linker adds no rpath, so build with:
#   RUSTFLAGS="-C link-arg=-Wl,-rpath,/Library/Developer/CommandLineTools/usr/lib" cargo install cargo-spellcheck --version 0.15.7 --locked
# (for Xcode instead of Command Line Tools use /Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib)
install-tools:
    cargo install cargo-spellcheck --version 0.15.7 --locked

# prepare a fresh clone: tools and git hooks
setup: tools
    pre-commit install

# ========== CI

# check formatting without changes
fmt-check:
    cargo +{{nightly}} fmt --check

# unused dependencies
shear-check:
    cargo shear

# every feature on its own
matrix:
    cargo hack check --workspace --each-feature --no-dev-deps

# install the pinned nightly
nightly-install:
    rustup toolchain install {{nightly}} --profile minimal -c rustfmt,clippy

# documentation
doc:
    cargo +{{nightly}} doc --workspace --no-deps --all-features

# tests with a JUnit report
test-ci:
    cargo nextest run --workspace --all-features --profile ci
    cargo test --workspace --doc

# coverage
coverage:
    cargo llvm-cov nextest --workspace --all-features --profile ci

# coverage for CI: cobertura and html
coverage-ci:
    cargo llvm-cov clean --workspace
    cargo llvm-cov nextest --workspace --no-report --all-features --profile ci
    cargo llvm-cov report --cobertura --output-path target/cobertura.xml
    cargo llvm-cov report --html --output-dir coverage
    @cargo llvm-cov report | awk '/^TOTAL/ { print "Coverage: " $10 }'
    cargo llvm-cov clean --workspace
