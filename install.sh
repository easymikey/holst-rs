#!/bin/sh
# Downloads holst and holst-mcp from a GitHub release and installs them.
#   HOLST_VERSION=v0.1.0   release tag (default: the latest release)
#   HOLST_INSTALL_DIR=DIR  target directory (default: ~/.local/bin)
set -eu

repo=easymikey/holst-rs
dir=${HOLST_INSTALL_DIR:-$HOME/.local/bin}
if [ -n "${HOLST_VERSION:-}" ]; then
    base=https://github.com/$repo/releases/download/$HOLST_VERSION
else
    base=https://github.com/$repo/releases/latest/download
fi
fallback="cargo install --git https://github.com/$repo --locked holst-cli holst-mcp"
linux_support='the Linux build runs on 64-bit x86_64 or aarch64 systems with glibc 2.35 or newer (Ubuntu 22.04, Debian 12 or later); musl systems such as Alpine are not supported'

fail() {
    printf 'install: %s\n' "$1" >&2
    printf 'install: to build from source instead (needs Rust, https://rustup.rs): %s\n' "$fallback" >&2
    exit 1
}

system=$(uname -s)
machine=$(uname -m)

case "$system/$machine" in
    Darwin/arm64) triple=aarch64-apple-darwin ;;
    Darwin/x86_64)
        # A shell under Rosetta reports x86_64 on Apple silicon; the native build suits it better.
        if [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || true)" = 1 ]; then
            triple=aarch64-apple-darwin
        else
            triple=x86_64-apple-darwin
        fi
        ;;
    Linux/x86_64) triple=x86_64-unknown-linux-gnu ;;
    Linux/aarch64 | Linux/arm64) triple=aarch64-unknown-linux-gnu ;;
    Darwin/* | Linux/*) fail "no holst build for $machine on $system" ;;
    *) fail "this script installs on macOS and Linux only; on Windows use install.ps1 or download the zip from the GitHub Releases page" ;;
esac

if [ "$system" = Linux ]; then
    libc=$(getconf GNU_LIBC_VERSION 2>/dev/null || true)
    case "$libc" in
        "glibc "*) ;;
        *) fail "this system does not use glibc; $linux_support" ;;
    esac
    version=${libc#glibc }
    major=${version%%.*}
    minor=${version#"$major"}
    minor=${minor#.}
    minor=${minor%%.*}
    case "$major/$minor" in
        */ | /* | *[!0-9/]*) fail "cannot read the glibc version from '$libc'; $linux_support" ;;
    esac
    if [ "$major" -lt 2 ] || { [ "$major" -eq 2 ] && [ "$minor" -lt 35 ]; }; then
        fail "glibc $version is too old; $linux_support"
    fi
    bits=$(getconf LONG_BIT 2>/dev/null || true)
    [ "$bits" = 64 ] || fail "this system has a ${bits:-unknown}-bit userland; $linux_support"
fi

if command -v sha256sum >/dev/null 2>&1; then
    sum='sha256sum'
elif command -v shasum >/dev/null 2>&1; then
    sum='shasum -a 256'
else
    fail "neither sha256sum nor shasum is installed, so the download cannot be checked"
fi

archive=holst-$triple.tar.gz
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

printf 'install: downloading %s\n' "$archive"
curl -fsSL -o "$tmp/$archive" "$base/$archive" || fail "cannot download $base/$archive"
curl -fsSL -o "$tmp/checksums.txt" "$base/checksums.txt" || fail "cannot download $base/checksums.txt"

expected=$(awk -v name="$archive" '$2 == name { print $1 }' "$tmp/checksums.txt")
[ -n "$expected" ] || fail "no checksum for $archive in checksums.txt"
actual=$($sum "$tmp/$archive" | awk '{ print $1 }')
[ "$expected" = "$actual" ] || fail "checksum mismatch for $archive"

tar -xzf "$tmp/$archive" -C "$tmp" holst holst-mcp
mkdir -p "$dir"
install -m 755 "$tmp/holst" "$dir/holst"
install -m 755 "$tmp/holst-mcp" "$dir/holst-mcp"

printf 'install: holst and holst-mcp installed to %s\n' "$dir"

case ":$PATH:" in
    *":$dir:"*) ;;
    *) printf 'install: warning: %s is not on your PATH; add it to your shell profile\n' "$dir" >&2 ;;
esac

echo "next:"
echo "  holst auth login"
# Single-quoted for the shell, so a path with spaces or $ can be pasted as is
mcp=$(printf '%s' "$dir/holst-mcp" | sed "s/'/'\\\\''/g")
echo "  claude mcp add holst-rs -s user -- '$mcp'"
