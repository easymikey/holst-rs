#!/bin/sh
# Cross-platform check in Docker: Linux (native arch, and linux/amd64 emulated),
# Linux keyring (no D-Bus, gnome-keyring, HOLST_HOME), scripts/install-from-source.sh, a Windows
# cross-build with tests under wine, and install.ps1 under PowerShell.
# Logins only ever go to the local mock from the checks directory; never to Holst.
#
#   scripts/cross-check.sh                   every group
#   scripts/cross-check.sh linux windows     only these groups
#
# Groups: linux keyring install amd64 windows powershell.
# Env: HOLST_XC_CHECKS     a checks directory holding cli-e2e.mjs and mock.py; not shipped
#      HOLST_XC_INSTALL_SH a source installer to test instead of the repo's one
#      HOLST_XC_AMD64=0    skip the emulated linux/amd64 group
#      HOLST_XC_LOGS       where step logs go (default: a temp directory)
# Cargo caches live in the named volumes holst-xc-cargo and holst-xc-target-*.
set -u

ROOT=$(cd "$(dirname "$0")/.." && pwd)
LOGS=${HOLST_XC_LOGS:-$(mktemp -d "${TMPDIR:-/tmp}/holst-xc.XXXXXX")}
mkdir -p "$LOGS"
SUMMARY="$LOGS/summary"
: >"$SUMMARY"

# The checks directory is not shipped: it comes from HOLST_XC_CHECKS only.
CHECKS=${HOLST_XC_CHECKS:-}

# Run step $1 (the rest is the command); log, time, show the tail; stop on failure.
step() {
	name=$1
	shift
	log="$LOGS/$name.log"
	printf '\n== %s\n' "$name"
	start=$(date +%s)
	"$@" >"$log" 2>&1
	code=$?
	took=$(($(date +%s) - start))
	grep -v -E '^ +(Compiling|Downloaded|Downloading|Checking|Installing|Fresh) ' "$log" | tail -n "${TAIL:-8}" | sed 's/^/   /'
	if [ "$code" -ne 0 ]; then
		printf '%-28s %-5s %5ss\n' "$name" FAIL "$took" >>"$SUMMARY"
		printf '\n'
		cat "$SUMMARY"
		echo "CROSS CHECK FAILED at step: $name (exit $code, log: $log)"
		exit 1
	fi
	result=ok
	grep -q '^skipped:' "$log" && result=skip
	printf '%-28s %-5s %5ss\n' "$name" "$result" "$took" >>"$SUMMARY"
	echo "   $result in ${took}s"
}

# A step that needs the checks directory.
need_checks() {
	[ -n "$CHECKS" ] || {
		echo "no checks directory found (a directory holding cli-e2e.mjs and mock.py, not shipped): set HOLST_XC_CHECKS"
		return 1
	}
}

# Run docker/linux-check.sh step $2 in the Linux image for platform $1.
linux() {
	platform=$1
	shift
	tag=holst-xc-linux
	[ "$platform" = linux/amd64 ] && tag=holst-xc-linux-amd64
	# /checks only when there is one: an empty source makes docker fail before
	# need_checks can explain; steps that need it run need_checks first.
	docker run --rm --platform "$platform" \
		-v "$ROOT:/src:ro" \
		${CHECKS:+-v "$CHECKS:/checks:ro"} \
		${HOLST_XC_INSTALL_SH:+-v "$HOLST_XC_INSTALL_SH:/extra/install.sh:ro"} \
		-v holst-xc-cargo:/usr/local/cargo/registry \
		-v "holst-xc-target-${platform#linux/}:/work/target" \
		"$tag" sh /src/docker/linux-check.sh "$@"
}

image() {
	docker build -q --platform "$1" -t "$2" -f "$ROOT/docker/$3" "$ROOT/docker"
}

native=linux/$(docker version --format '{{.Server.Arch}}' 2>/dev/null || echo amd64)
groups=${*:-linux keyring install amd64 windows powershell}
echo "cross check: $ROOT"
echo "checks: ${CHECKS:-none}; logs: $LOGS; native platform: $native"

for group in $groups; do
	case $group in
	linux)
		step linux-checks need_checks
		step linux-image image "$native" holst-xc-linux linux.Dockerfile
		step linux-build linux "$native" build
		step linux-test linux "$native" test
		step linux-e2e linux "$native" e2e
		;;
	keyring)
		step keyring-checks need_checks
		step keyring-image image "$native" holst-xc-linux linux.Dockerfile
		step keyring-no-dbus linux "$native" keyring-none
		step keyring-no-provider linux "$native" keyring-no-provider
		step keyring-gnome-keyring linux "$native" keyring-dbus
		step keyring-holst-home linux "$native" keyring-file
		;;
	install)
		step install-image image "$native" holst-xc-linux linux.Dockerfile
		TAIL=12 step install-sh linux "$native" install
		;;
	amd64)
		[ "${HOLST_XC_AMD64:-1}" = 0 ] || [ "$native" = linux/amd64 ] && {
			echo "skipped: linux/amd64 (HOLST_XC_AMD64=0 or native)"
			continue
		}
		step amd64-checks need_checks
		step amd64-image image linux/amd64 holst-xc-linux-amd64 linux.Dockerfile
		step amd64-build linux linux/amd64 build
		step amd64-test linux linux/amd64 test
		step amd64-e2e linux linux/amd64 e2e
		;;
	windows)
		step windows-image image linux/amd64 holst-xc-windows windows.Dockerfile
		TAIL=24 step windows-gnu docker run --rm --platform linux/amd64 \
			-v "$ROOT:/src:ro" -v holst-xc-cargo:/usr/local/cargo/registry \
			-v holst-xc-target-win:/work/target \
			holst-xc-windows sh /src/docker/windows-check.sh
		;;
	powershell)
		step powershell-image image "$native" holst-xc-powershell powershell.Dockerfile
		TAIL=24 step install-ps1 docker run --rm -v "$ROOT:/src:ro" holst-xc-powershell sh /src/docker/powershell-check.sh
		;;
	*)
		echo "unknown group '$group'"
		exit 2
		;;
	esac
done

printf '\n'
cat "$SUMMARY"
echo "CROSS CHECK OK"
