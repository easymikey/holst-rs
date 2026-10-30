nightly := "nightly-2026-10-30"
size_holst := "11400000"
size_holst_mcp := "12400000"
startup_ms_holst := "20"
startup_ms_holst_mcp := "20"

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
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
    cargo bin cargo-nextest run --workspace --all-features --locked --profile ci
    cargo test --workspace --all-features --locked --doc

# CI job `deps`: licences and advisories, unused dependencies, each feature on its own.
deps:
    cargo bin cargo-deny --config .config/deny.toml check
    cargo bin cargo-machete
    cargo bin cargo-hack check --workspace --each-feature --locked

# Every CI job.
ci: lint deps check
# CI job `budget`: release binary sizes and `--version` startup against the limits at the top.
budget:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --release --locked -p holst-cli -p holst-mcp
    if [ "$(uname)" = Darwin ]; then size() { stat -f%z "$1"; }; else size() { stat -c%s "$1"; }; fi
    median_ms() {
        perl -MTime::HiRes=time -e '
            my @t;
            for (1..20) {
                my $start = time;
                system($ARGV[0], "--version") == 0 or die "$ARGV[0] --version failed\n";
                push @t, (time - $start) * 1000;
            }
            @t = sort { $a <=> $b } @t;
            printf STDERR "%.1f", ($t[9] + $t[10]) / 2;
        ' "$1" 2>&1 >/dev/null
    }
    failed=0
    check() {
        local bin=target/release/$1 size_limit=$2 startup_limit=$3 bytes ms
        bytes=$(size "$bin")
        ms=$(median_ms "$bin")
        echo "$1: size $bytes B (limit $size_limit), startup median $ms ms (limit $startup_limit)"
        if [ "$bytes" -gt "$size_limit" ]; then echo "$1: over the size limit" >&2; failed=1; fi
        if awk -v a="$ms" -v b="$startup_limit" 'BEGIN { exit !(a > b) }'; then echo "$1: over the startup limit" >&2; failed=1; fi
    }
    check holst {{ size_holst }} {{ startup_ms_holst }}
    check holst-mcp {{ size_holst_mcp }} {{ startup_ms_holst_mcp }}
    exit $failed
