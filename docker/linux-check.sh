#!/bin/sh
# One Linux check step inside the holst-xc-linux image:
#   build | test | e2e | keyring-none | keyring-dbus | keyring-file
# Mounts: /src (repo, read-only), /checks (a checks directory holding cli-e2e.mjs and mock.py, given by HOLST_XC_CHECKS; not shipped; read-only),
# /work/target (cargo target volume). Every login goes to the local mock.
set -eu

rsync -a --delete --exclude /target --exclude /.claude --exclude /.git --exclude /.scratch /src/ /work/
cd /work
BIN=/work/target/debug/holst
PORT=8765
export NO_COLOR=1 HOLST_BASE_URL="http://127.0.0.1:$PORT"

fail() {
	echo "FAIL: $*"
	exit 1
}

# Start the mock and wait until it answers.
mock() {
	python3 /checks/mock.py "$PORT" &
	for _ in 1 2 3 4 5 6 7 8 9 10; do
		python3 -c "import socket; socket.create_connection(('127.0.0.1', $PORT), 1)" 2>/dev/null && return 0
		sleep 0.3
	done
	fail "the mock did not start"
}

# Run holst with the mock login on stdin; stderr in $OUT, exit code in $CODE.
login() {
	CODE=0
	OUT=$(printf 'mock@example.com\n123456\n' | "$BIN" login 2>&1 >/dev/null) || CODE=$?
}

# Run a holst command; stdout and stderr in $OUT, exit code in $CODE.
run() {
	CODE=0
	OUT=$("$BIN" "$@" 2>&1) || CODE=$?
}

# $OUT matches the extended regex $1, or the step fails with $2.
has() {
	printf '%s\n' "$OUT" | grep -Eq -- "$1" || fail "$2; output was:
$OUT"
}

# $OUT does not match $1.
lacks() {
	printf '%s\n' "$OUT" | grep -Eq -- "$1" && fail "$2; output was:
$OUT"
	return 0
}

# No panic, backtrace, tracing line or Rust debug structure in $OUT.
clean() {
	lacks 'panicked|RUST_BACKTRACE|Caused by|\bError:| WARN | ERROR |Os \{|kind: ' "debug noise"
}

code() {
	[ "$CODE" -eq "$1" ] || fail "exit code $CODE, want $1; output was:
$OUT"
}

case "${1:-}" in
build)
	cargo build --workspace --locked
	;;
test)
	rc=0
	cargo test --workspace --locked >/tmp/test.log 2>&1 || rc=$?
	[ "$rc" -eq 0 ] || {
		grep -E 'FAILED|panicked|^error' /tmp/test.log | head -40
		fail "cargo test failed"
	}
	awk '/^test result: ok/ { runs++; passed += $4 } END { printf "%d test runs, %d passed, 0 failed\n", runs, passed }' /tmp/test.log
	;;
e2e)
	E2E_ROOT=/work node /checks/cli-e2e.mjs
	;;
keyring-none | keyring-no-provider)
	# No D-Bus session at all, or a session bus with no Secret Service provider:
	# the login is refused, not put in a file.
	if [ "$1" = keyring-no-provider ] && [ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]; then
		rm -f /usr/share/dbus-1/services/org.freedesktop.secrets.service \
			/usr/share/dbus-1/services/org.gnome.keyring.service
		exec dbus-run-session -- sh "$0" "$@"
	fi
	[ "$1" = keyring-none ] && unset DBUS_SESSION_BUS_ADDRESS
	unset HOLST_HOME
	mock
	login
	echo "login: exit $CODE"
	printf '%s\n' "$OUT" | grep -E '^└|^ +(cause|hint):' || true
	code 1
	has '^└  no Secret Service found \(gnome-keyring or KWallet\)$' "no clear error"
	has '^   hint: start gnome-keyring or KWallet, or set `HOLST_HOME=<dir>` to keep the login in `<dir>/session`$' "no HOLST_HOME hint"
	clean
	[ ! -e "$HOME/.config/holst/token" ] || fail "the login went to ~/.config/holst/token"
	run whoami
	echo "whoami: exit $CODE"
	printf '%s\n' "$OUT"
	code 1
	has '^error: no Secret Service found \(gnome-keyring or KWallet\)$' "no clear error"
	has '^hint: start gnome-keyring or KWallet, or set `HOLST_HOME=<dir>` to keep the login in `<dir>/session`$' "no HOLST_HOME hint"
	clean
	# A login an older holst kept in ~/.config/holst/token still works, and the
	# cookies a request rotates go back to that file, not to an error.
	legacy=$(mktemp -d)
	CODE=0
	OUT=$(printf 'mock@example.com\n123456\n' | HOLST_HOME=$legacy "$BIN" login 2>&1 >/dev/null) || CODE=$?
	code 0
	mkdir -p "$HOME/.config/holst"
	cp "$legacy/session" "$HOME/.config/holst/token"
	cp "$legacy/session" "$legacy/before"
	run whoami
	echo "whoami with the legacy file: exit $CODE"
	code 0
	has '"name": "Mock User"' "whoami did not read the legacy file"
	run api POST /gapi/login/with_email --body '{"email":"mock@example.com","password":"123456"}'
	echo "rotation with the legacy file: exit $CODE"
	code 0
	lacks 'warning|error' "the rotated cookies were not saved"
	cmp -s "$legacy/before" "$HOME/.config/holst/token" && fail "the rotated cookies did not reach the legacy file"
	run logout
	echo "logout with the legacy file: exit $CODE: $OUT"
	code 0
	has 'forgot the login in: file' "logout did not forget the legacy file"
	[ ! -e "$HOME/.config/holst/token" ] || fail "logout left the legacy file"
	;;
keyring-dbus)
	# A headless D-Bus session with gnome-keyring unlocked by a test password.
	if [ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]; then
		export XDG_RUNTIME_DIR=/tmp/xdg
		mkdir -p -m 700 "$XDG_RUNTIME_DIR"
		exec dbus-run-session -- sh "$0" "$@"
	fi
	unset HOLST_HOME
	printf 'test-password' | gnome-keyring-daemon --unlock --components=secrets >/dev/null
	mock
	login
	echo "login: exit $CODE"
	code 0
	has 'saved to Secret Service' "not saved to Secret Service"
	clean
	[ ! -e "$HOME/.config/holst/token" ] || fail "the login also went to the file"
	secret-tool search service holst-rs 2>&1 | grep -q 'username = session-token' || fail "no item in Secret Service"
	echo "Secret Service item: service=holst-rs username=session-token"
	run whoami
	echo "whoami: exit $CODE"
	code 0
	has '"name": "Mock User"' "whoami did not answer"
	run logout
	echo "logout: exit $CODE: $OUT"
	code 0
	has 'forgot the login in: Secret Service' "logout did not forget the Secret Service login"
	[ -z "$(secret-tool search service holst-rs 2>&1)" ] || fail "the item is still in Secret Service"
	run whoami
	echo "whoami after logout: exit $CODE"
	code 1
	has '^error: no login found' "whoami after logout"
	;;
keyring-file)
	# HOLST_HOME: a 0600 file, Secret Service never touched (no D-Bus here).
	unset DBUS_SESSION_BUS_ADDRESS
	HOLST_HOME=$(mktemp -d)
	export HOLST_HOME
	mock
	login
	echo "login: exit $CODE"
	code 0
	has "saved to $HOLST_HOME/session" "not saved to the HOLST_HOME file"
	clean
	mode=$(stat -c '%a' "$HOLST_HOME/session")
	echo "file mode: $mode"
	[ "$mode" = 600 ] || fail "the session file is $mode, want 600"
	run whoami
	echo "whoami: exit $CODE"
	code 0
	has '"name": "Mock User"' "whoami did not answer"
	run logout
	echo "logout: exit $CODE: $OUT"
	code 0
	[ ! -e "$HOLST_HOME/session" ] || fail "logout left the session file"
	;;
*)
	fail "unknown step '${1:-}'"
	;;
esac
echo "STEP OK: $1"
