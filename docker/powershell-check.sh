#!/bin/sh
# install.ps1 check inside the holst-xc-powershell image: parser errors,
# PSScriptAnalyzer, and the script's flow with a stub cargo.
# Mounts: /src (repo, read-only).
set -eu
PS1=/src/install.ps1

fail() {
	echo "FAIL: $*"
	exit 1
}

[ -f "$PS1" ] || fail "no install.ps1"
pwsh -NoLogo -NoProfile -Command '$PSVersionTable.PSVersion.ToString()' | sed 's/^/pwsh /'

echo "== parse and PSScriptAnalyzer"
pwsh -NoLogo -NoProfile -File /src/docker/powershell-lint.ps1 -Path "$PS1" || fail "install.ps1 has parser errors or analyzer findings"

# A cargo that logs its arguments and exits with $CARGO_STUB_EXIT.
stub=$(mktemp -d)
cat >"$stub/cargo" <<'EOF'
#!/bin/sh
echo "stub cargo $*" >>"$CARGO_STUB_LOG"
exit "${CARGO_STUB_EXIT:-0}"
EOF
chmod +x "$stub/cargo"
export CARGO_STUB_LOG="$stub/log" CARGO_HOME=/opt/cargo-home NO_COLOR=1

run() {
	code=0
	out=$(pwsh -NoLogo -NoProfile -File "$PS1" 2>&1) || code=$?
	printf '%s\n' "$out" | sed 's/^/  | /'
}

echo "== flow: cargo works"
PATH="$stub:/opt/pwsh:/usr/bin:/bin" run
[ "$code" -eq 0 ] || fail "exit $code, want 0"
grep -qx 'stub cargo install --path crates/holst-cli --locked' "$CARGO_STUB_LOG" || fail "holst-cli not installed"
grep -qx 'stub cargo install --path crates/holst-mcp --locked' "$CARGO_STUB_LOG" || fail "holst-mcp not installed"
printf '%s\n' "$out" | grep -q "installed: /opt/cargo-home/bin/holst.exe, /opt/cargo-home/bin/holst-mcp.exe" || fail "no install location"
printf '%s\n' "$out" | grep -q 'is not on PATH' || fail "no PATH note"
printf '%s\n' "$out" | grep -q '^  holst login$' || fail "no next step: holst login"
printf '%s\n' "$out" | grep -q '^  claude mcp add holst-rs -s user -- ' || fail "no next step: claude mcp add"

echo "== flow: cargo fails"
CARGO_STUB_EXIT=101 PATH="$stub:/opt/pwsh:/usr/bin:/bin" run
[ "$code" -eq 1 ] || fail "exit $code, want 1"
printf '%s\n' "$out" | grep -q '^error: cargo install --path crates/holst-cli --locked failed with exit code 101$' || fail "no clear error"

echo "== flow: no cargo"
PATH="/opt/pwsh:/usr/bin:/bin" run
[ "$code" -eq 1 ] || fail "exit $code, want 1"
printf '%s\n' "$out" | grep -q '^error: cargo not found' || fail "no clear error"

echo "POWERSHELL OK"
