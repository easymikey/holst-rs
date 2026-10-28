#!/bin/sh
# Windows check inside the holst-xc-windows image: cross-build for
# x86_64-pc-windows-gnu, build the tests, run them and `holst.exe` under wine.
# Mounts: /src (repo, read-only), /work/target (cargo target volume).
set -eu
T=x86_64-pc-windows-gnu
DEPS=/work/target/$T/debug/deps
step() { printf '\n== %s\n' "$*"; }

rsync -a --delete --exclude /target --exclude /.claude --exclude /.git --exclude /.scratch /src/ /work/
cd /work

step "windows: cargo build --workspace --locked --target $T"
cargo build --workspace --locked --target "$T"
ls -l "target/$T/debug/holst.exe" "target/$T/debug/holst-mcp.exe"

step "windows: keyring backend is windows-native"
cargo tree -p holst-core --target "$T" -e features -i keyring | grep -E 'keyring feature "windows-native"'

step "windows: cargo test --workspace --no-run --target $T"
cargo test --workspace --locked --no-run --target "$T" 2>&1 | grep -E 'Finished|Executable'

step "windows: wine holst.exe --help, holst-mcp.exe --help"
wineboot -i >/dev/null 2>&1 || true
wine "target/$T/debug/holst.exe" --help >/tmp/help.txt 2>/dev/null
head -5 /tmp/help.txt
grep -q 'holst login' /tmp/help.txt
wine "target/$T/debug/holst-mcp.exe" --help >/tmp/mcp-help.txt 2>&1
head -2 /tmp/mcp-help.txt
grep -q 'holst-mcp' /tmp/mcp-help.txt

step "windows: unit tests under wine"
# insta finds the workspace with `cargo metadata`, which wine cannot run.
# Skipped as wine limits, not checked on real Windows:
# - near_fs_limit: wine fails a hard link to a 240-byte UTF-8 name (NotFound);
#   NTFS counts that name as 122 UTF-16 units.
# - racing_writes_never_tear_the_file: wine's rename over a file another thread
#   renames gives Access denied; real Windows 10+ renames with POSIX semantics.
rc=0
INSTA_WORKSPACE_ROOT=/work cargo test --workspace --locked --target "$T" --lib --bins --no-fail-fast \
	-- --skip near_fs_limit --skip racing_writes_never_tear_the_file >/tmp/wine-tests.log 2>&1 || rc=$?
grep -E '^ +Running|^test result' /tmp/wine-tests.log
[ "$rc" -eq 0 ] || {
	sed -n '/^failures:$/,/^test result/p' /tmp/wine-tests.log
	exit 1
}

step "windows: integration tests under wine, one by one (informational)"
# wine under Rosetta (Docker Desktop on Apple silicon) kills processes that run
# a local server or spawn holst.exe ("rosetta error: invalid gdt selector"), so
# these results do not fail the step; each test gets three tries.
newest() { ls -t "$DEPS"/"$1"-*.exe 2>/dev/null | head -1; }
failed=0
crashed=0
for suite in e2e resolve transport pty; do
	exe=$(newest "$suite")
	[ -n "$exe" ] || continue
	names=$(INSTA_WORKSPACE_ROOT=/work wine "$exe" --list --format terse 2>/dev/null | sed -n 's/: test$//p')
	passed=0
	for name in $names; do
		ok=0
		for attempt in 1 2 3; do
			INSTA_WORKSPACE_ROOT=/work wine "$exe" --exact "$name" >/tmp/one.log 2>&1 && {
				ok=$attempt
				break
			}
		done
		if [ "$ok" -eq 0 ] && grep -q 'rosetta error\|Trace/breakpoint trap' /tmp/one.log; then
			crashed=$((crashed + 1))
		elif [ "$ok" -eq 0 ]; then
			echo "failed $suite::$name"
			sed -n '/^---- /,/^failures:$/p' /tmp/one.log | grep -v '^$' | head -12
			failed=$((failed + 1))
		else
			passed=$((passed + 1))
			[ "$ok" -eq 1 ] || echo "retried $suite::$name: passed on attempt $ok"
		fi
	done
	echo "$suite: $passed passed"
done
echo "integration under wine: $failed failed, $crashed crashed by the emulator (not counted)"
echo "WINDOWS OK"
