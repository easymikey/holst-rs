#!/bin/sh
# Builds holst and holst-mcp and installs them into Cargo's bin directory.
# On macOS, HOLST_SIGN_IDENTITY=<name or SHA-1> signs both with an existing code-signing identity.
set -eu

cd "$(dirname "$0")/.."

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    red=$(printf '\033[31m')
    green=$(printf '\033[32m')
    yellow=$(printf '\033[33m')
    reset=$(printf '\033[0m')
else
    red='' green='' yellow='' reset=''
fi

step() { printf '%s==>%s %s\n' "$green" "$reset" "$*"; }
note() { printf '%snote:%s %s\n' "$yellow" "$reset" "$*"; }
die() {
    printf '%serror:%s %s\n' "$red" "$reset" "$*" >&2
    exit 1
}

command -v cargo >/dev/null 2>&1 ||
    die "cargo not found: install Rust from https://rustup.rs, then open a new terminal"

macos=false
[ "$(uname -s)" = Darwin ] && macos=true
identity=${HOLST_SIGN_IDENTITY:-}

# Fail before the build, not after it
if $macos && [ -n "$identity" ]; then
    for tool in security codesign; do
        command -v "$tool" >/dev/null 2>&1 ||
            die "$tool not found: install the Xcode Command Line Tools (xcode-select --install)"
    done
    security find-identity -v -p codesigning | grep -qF -- "$identity" ||
        die "no valid code-signing identity \"$identity\" in your keychains (security find-identity -v -p codesigning)"
fi

bin="${CARGO_INSTALL_ROOT:-${CARGO_HOME:-$HOME/.cargo}}/bin"

step "building and installing holst"
cargo install --path crates/holst-cli --locked
step "building and installing holst-mcp"
cargo install --path crates/holst-mcp --locked

if $macos && [ -n "$identity" ]; then
    step "signing with \"$identity\""
    codesign --force --timestamp=none --sign "$identity" --identifier so.holst.cli "$bin/holst"
    codesign --force --timestamp=none --sign "$identity" --identifier so.holst.mcp "$bin/holst-mcp"
fi

echo
step "installed: $bin/holst, $bin/holst-mcp"
case ":$PATH:" in
*":$bin:"*) ;;
*) note "$bin is not on PATH: add it in your shell profile" ;;
esac
echo "next:"
echo "  holst auth login"
# Single-quoted for the shell, so a path with spaces or $ can be pasted as is
mcp=$(printf '%s' "$bin/holst-mcp" | sed "s/'/'\\\\''/g")
echo "  claude mcp add holst-rs -s user -- '$mcp'"
