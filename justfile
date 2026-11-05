nightly := "nightly-2026-10-30"
zig := "0.17.0"
size_holst := "11400000"
size_holst_mcp := "12400000"
startup_ms_holst := "50"
startup_ms_holst_mcp := "50"

default: lint check deps features budget

lint: fmt-check spell clippy
check: test doctest doc
deps: deny machete

fmt:
    cargo +{{ nightly }} fmt --all

fmt-check:
    cargo +{{ nightly }} fmt --all --check

spell:
    cargo bin typos

clippy:
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

test:
    cargo bin cargo-nextest run --workspace --all-features --locked

doctest:
    cargo test --workspace --all-features --locked --doc

doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked

deny:
    cargo bin cargo-deny --config .config/deny.toml check

machete:
    cargo bin cargo-machete

commits range="origin/main..HEAD":
    #!/usr/bin/env bash
    set -euo pipefail
    git log --no-merges --format=%s "{{ range }}" | {
        status=0
        while IFS= read -r subject; do
            printf '%s\n' "$subject" | .githooks/commit-msg /dev/stdin || status=1
        done
        exit "$status"
    }
ci-setup:
    #!/usr/bin/env bash
    set -euo pipefail
    rustup toolchain install {{ nightly }} --profile minimal --component rustfmt
    name=zig-$(uname -m)-linux-{{ zig }}
    curl -fsSL "https://ziglang.org/download/{{ zig }}/$name.tar.xz" | tar -xJ -C "$RUNNER_TEMP"
    echo "$RUNNER_TEMP/$name" >> "$GITHUB_PATH"

hooks:
    git config core.hooksPath .githooks

# Checks, the semver check, then a dry run of the release. Pass `execute` to do it. The dry run skips the package build: the crates still carry the published versions. Below 1.0 only a patch level may not break the API.
ship level execute="":
    cargo bin just default
    cargo bin cargo-semver-checks check-release --workspace --release-type {{ if level == "patch" { "patch" } else { "major" } }}
    cargo bin cargo-release {{ level }} --workspace {{ if execute == "execute" { "--execute --no-confirm --no-push" } else { "--no-verify" } }}
    {{ if execute == "execute" { "cargo bin just push-release" } else { "" } }}

# GitHub starts no workflow for more than three tags in one push: push main, then each tag of HEAD alone.
push-release:
    git push origin main
    for tag in $(git tag --points-at HEAD); do git push origin "refs/tags/$tag"; done

# Print the changelog section of a tag such as holst-cli-v0.5.0.
release-notes tag:
    #!/usr/bin/env bash
    set -euo pipefail
    tag="{{ tag }}"
    crate="${tag%-v*}"
    version="${tag##*-v}"
    if [ "$crate" = "$tag" ] || [ -z "$crate" ] || [ -z "$version" ]; then
        echo "release-notes: '$tag' is not <crate>-v<version>" >&2
        exit 1
    fi
    file=CHANGELOG.md
    notes="$(awk -v head="## $version (" '
        /^## / { if (found) exit; found = index($0, head) == 1; next }
        found { print }
    ' "$file")"
    if [ -z "$(printf '%s' "$notes" | tr -d '[:space:]')" ]; then
        echo "release-notes: no notes for $version in $file" >&2
        exit 1
    fi
    printf '%s\n' "$notes" | awk 'NF { started = 1 } started'

features:
    cargo bin cargo-hack check --workspace --each-feature --locked

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

dist target:
    #!/usr/bin/env bash
    set -euo pipefail
    target={{ target }}
    out=${CARGO_TARGET_DIR:-target}/$target/release
    ext=""
    case "$target" in
        *-linux-musl) build=(cargo bin cargo-zigbuild) ;;
        *-windows-msvc)
            ext=".exe"
            if [[ ${OSTYPE:-} == msys* ]]; then build=(cargo build); else build=(cargo bin cargo-xwin build); fi
            ;;
        *) build=(cargo build) ;;
    esac
    rustup target add "$target"
    "${build[@]}" --release --locked -p holst-cli -p holst-mcp --target "$target"
    if [[ $target == *-linux-musl ]]; then
        for bin in holst holst-mcp; do
            info=$(file "$out/$bin")
            echo "$info"
            case "$info" in
                *"statically linked"* | *"static-pie linked"*) ;;
                *) echo "$bin is not statically linked" >&2; exit 1 ;;
            esac
        done
    fi
    if [ "$target" = x86_64-unknown-linux-musl ] && [ "$(uname -sm)" = "Linux x86_64" ]; then
        "$out/holst" --version
        "$out/holst-mcp" --version
    fi
    sum() { if command -v sha256sum >/dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }
    pack() {
        local package=$1 binary=$2 stage=dist/stage/$1 archive
        mkdir -p "$stage"
        cp "$out/$binary$ext" LICENSE-MIT LICENSE-APACHE readme.md "$stage/"
        if [ -n "$ext" ]; then
            archive=$package-$target.zip
            if command -v zip >/dev/null; then
                (cd "$stage" && zip -q -X "../../$archive" "$binary$ext" LICENSE-MIT LICENSE-APACHE readme.md)
            else
                (cd "$stage" && pwsh -NoProfile -Command "Compress-Archive -Path * -DestinationPath ../../$archive")
            fi
        else
            archive=$package-$target.tar.gz
            COPYFILE_DISABLE=1 tar -czf "dist/$archive" -C "$stage" "$binary" LICENSE-MIT LICENSE-APACHE readme.md
        fi
        (cd dist && sum "$archive" > "$archive.sha256")
    }
    rm -rf dist
    mkdir -p dist
    pack holst-cli holst
    pack holst-mcp holst-mcp
    rm -rf dist/stage
    ls dist

# Attach the archives `dist` built for a target to the GitHub release of a holst-cli or holst-mcp tag.
upload tag target:
    #!/usr/bin/env bash
    set -euo pipefail
    tag={{ tag }}
    gh release upload "$tag" dist/"${tag%-v*}"-{{ target }}.* --clobber

homebrew tag:
    #!/usr/bin/env bash
    set -euo pipefail
    tag={{ tag }}
    package=${tag%-v*}
    version=${tag##*-v}
    case "$package" in
        holst-cli) class=HolstCli; desc="Command line for Holst whiteboards"; binary=holst ;;
        holst-mcp) class=HolstMcp; desc="MCP server for Holst whiteboards"; binary=holst-mcp ;;
        *) echo "$tag is not a holst-cli or holst-mcp tag" >&2; exit 1 ;;
    esac
    base=https://github.com/easymikey/holst-rs/releases/download/$tag
    sha() { curl -fsSL "$base/$package-$1.tar.gz.sha256" | awk '{ print $1 }'; }
    arm_mac_sha=$(sha aarch64-apple-darwin)
    intel_mac_sha=$(sha x86_64-apple-darwin)
    arm_linux_sha=$(sha aarch64-unknown-linux-musl)
    intel_linux_sha=$(sha x86_64-unknown-linux-musl)
    if [ -n "${TAP_DEPLOY_KEY:-}" ]; then
        mkdir -p ~/.ssh
        ssh-keyscan github.com >> ~/.ssh/known_hosts
        eval "$(ssh-agent -s)"
        printf '%s\n' "$TAP_DEPLOY_KEY" | tr -d '\r' | ssh-add -
    fi
    tap=$(mktemp -d)
    git clone git@github.com:easymikey/homebrew-holst.git "$tap"
    mkdir -p "$tap/Formula"
    cat > "$tap/Formula/$package.rb" <<EOF
    class $class < Formula
      desc "$desc"
      homepage "https://github.com/easymikey/holst-rs"
      version "$version"
      license any_of: ["MIT", "Apache-2.0"]

      on_macos do
        on_arm do
          url "$base/$package-aarch64-apple-darwin.tar.gz"
          sha256 "$arm_mac_sha"
        end

        on_intel do
          url "$base/$package-x86_64-apple-darwin.tar.gz"
          sha256 "$intel_mac_sha"
        end
      end

      on_linux do
        on_arm do
          url "$base/$package-aarch64-unknown-linux-musl.tar.gz"
          sha256 "$arm_linux_sha"
        end

        on_intel do
          url "$base/$package-x86_64-unknown-linux-musl.tar.gz"
          sha256 "$intel_linux_sha"
        end
      end

      def install
        bin.install "$binary"
      end

      test do
        assert_match version.to_s, shell_output("#{bin}/$binary --version")
      end
    end
    EOF
    cd "$tap"
    git config user.name "github-actions[bot]"
    git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
    git add "Formula/$package.rb"
    if git diff --cached --quiet; then
        echo "formula already up to date"
    else
        git commit -m "$package $version"
        git push origin HEAD:main
    fi
