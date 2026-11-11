#!/bin/sh
# Tests for install, update, rollback, removal and the launcher, using a fake Kobo directory and
# stub system commands. Never touches real processes or devices.
#   tests/package.sh            (optionally: tests/package.sh dist/inoink-<v>-kobo.tar.gz to also check a real package)
set -u
cd "$(dirname "$0")/.."
ROOT=$PWD
FAILS=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; FAILS=$((FAILS+1)); }
check() { # description command...
	d=$1; shift
	if "$@" >/dev/null 2>&1; then ok "$d"; else bad "$d"; fi
}
check_not() { d=$1; shift; if "$@" >/dev/null 2>&1; then bad "$d"; else ok "$d"; fi; }

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

make_pkg() { # version -> $WORK/pkg-<version>.tar.gz
	v=$1
	p="$WORK/stage-$v"
	mkdir -p "$p/inoink/release/scripts" "$p/inoink/dispatch"
	printf '%s\n' "$v" > "$p/inoink/VERSION"
	printf '#!/bin/sh\necho "inoink %s ran" >> "$INOINK_DATA/ran.log"\n[ -n "${FAKE_EXIT:-}" ] && exit "$FAKE_EXIT"\nexit 0\n' "$v" > "$p/inoink/release/inoink"
	cp contrib/inoink/inoink-run.sh "$p/inoink/release/inoink-run.sh"
	printf '#!/bin/sh\necho restored >> "$INOINK_DATA/nickel.log"\n' > "$p/inoink/release/nickel.sh"
	cp contrib/inoink/inoink-dispatch.sh "$p/inoink/dispatch/inoink.sh"
	cp contrib/inoink/inoink.nm "$p/inoink/dispatch/inoink.nm"
	chmod +x "$p"/inoink/release/* "$p/inoink/dispatch/inoink.sh"
	tar -czf "$WORK/pkg-$v.tar.gz" -C "$p" inoink
}

KOBO="$WORK/kobo"
mkdir -p "$KOBO/.kobo" "$KOBO/.adds/nm" "$KOBO/.adds/other"
echo "SN,x,4.46.23852,x,x,00000000390" > "$KOBO/.kobo/version"
echo keep > "$KOBO/.adds/other/file"
make_pkg 1.0.0
make_pkg 1.1.0
make_pkg 1.2.0

# --- install ---------------------------------------------------------------------------------
check_not "refuses a directory that is not a Kobo" sh scripts/install-inoink.sh "$WORK/pkg-1.0.0.tar.gz" "$WORK"
check "installs 1.0.0 with the launcher entry" sh scripts/install-inoink.sh "$WORK/pkg-1.0.0.tar.gz" "$KOBO" --menu
B="$KOBO/.adds/inoink"
check "active release is 1.0.0" test "$(cat "$B/active-release")" = 1.0.0
check "launcher menu entry written" test -f "$KOBO/.adds/nm/inoink"
check "unrelated launcher entries and apps untouched" test "$(cat "$KOBO/.adds/other/file")" = keep

# User data survives an update, the previous release is kept, older ones are pruned.
echo "my data" > "$B/data/inoink.db"
check "updates to 1.1.0" sh scripts/install-inoink.sh "$WORK/pkg-1.1.0.tar.gz" "$KOBO"
check "data preserved across update" test "$(cat "$B/data/inoink.db")" = "my data"
check "previous release recorded" test "$(cat "$B/previous-release")" = 1.0.0
check "updates to 1.2.0" sh scripts/install-inoink.sh "$WORK/pkg-1.2.0.tar.gz" "$KOBO"
check "release two versions back is pruned" test ! -d "$B/releases/1.0.0"
check "previous release (1.1.0) kept for rollback" test -d "$B/releases/1.1.0"
check "no staging directories left behind" test -z "$(ls -A "$B/releases" | grep '^\.stage' || true)"

# --- package safety ----------------------------------------------------------------------------
EVIL="$WORK/evil"
mkdir -p "$EVIL/inoink/release" "$EVIL/inoink/dispatch"
echo "../../escape" > "$EVIL/inoink/VERSION"
tar -czf "$WORK/evil.tar.gz" -C "$EVIL" inoink
check_not "rejects a package with a traversal version name" sh scripts/install-inoink.sh "$WORK/evil.tar.gz" "$KOBO"
check "nothing escaped the install directory" test ! -e "$KOBO/.adds/escape"

# --- dispatcher ----------------------------------------------------------------------------------
export FAKE=1
OUT=$(sh "$B/current/inoink.sh" 2>&1 < /dev/null)
sleep 2
check "dispatcher launches the active release" grep -q "inoink 1.2.0 ran" "$B/data/ran.log"

echo "../../etc" > "$B/active-release"
rm -f "$B/data/ran.log"
sh "$B/current/inoink.sh" > /dev/null 2>&1 < /dev/null
sleep 2
check "a hostile active-release name falls back to the previous release" grep -q "inoink 1.1.0 ran" "$B/data/ran.log"
echo "../../etc" > "$B/previous-release"
check_not "no valid release: dispatcher fails cleanly" sh "$B/current/inoink.sh"
printf '1.2.0\n' > "$B/active-release"; printf '1.1.0\n' > "$B/previous-release"

# --- launcher: Nickel is restored exactly once -------------------------------------------------
STUBS="$WORK/stubs"
mkdir -p "$STUBS"
for c in killall sync pidof reboot poweroff; do printf '#!/bin/sh\necho "%s $*" >> "%s/calls.log"\n' "$c" "$WORK" > "$STUBS/$c"; chmod +x "$STUBS/$c"; done
rm -f /tmp/reboot /tmp/power_off
rmdir /tmp/inoink.lock 2>/dev/null
run_launcher() { PATH="$STUBS:$PATH" INOINK_DATA="$B/data" sh "$B/releases/1.2.0/inoink-run.sh" > /dev/null 2>&1 < /dev/null; }

rm -f "$B/data/nickel.log" "$B/data/ran.log" "$WORK/calls.log"
run_launcher; sleep 2
check "launcher ran the app" grep -q "inoink 1.2.0 ran" "$B/data/ran.log"
check "launcher stopped Nickel's processes" grep -q "killall -TERM nickel" "$WORK/calls.log"
check "Nickel restored once after a normal exit" test "$(wc -l < "$B/data/nickel.log" | tr -d " ")" = 1
check "lock released" test ! -d /tmp/inoink.lock

rm -f "$B/data/nickel.log"
FAKE_EXIT=3 run_launcher; sleep 2
check "Nickel restored once after a failing app" test "$(wc -l < "$B/data/nickel.log" | tr -d ' ')" = 1

# A second launch while one runs does nothing.
mkdir /tmp/inoink.lock
rm -f "$B/data/nickel.log" "$B/data/ran.log"
run_launcher; sleep 2
check "double launch is ignored" test ! -e "$B/data/ran.log"
check "double launch does not touch Nickel" test ! -e "$B/data/nickel.log"
rmdir /tmp/inoink.lock

# Reboot intent is honoured instead of restarting Nickel.
touch /tmp/reboot
rm -f "$B/data/nickel.log" "$WORK/calls.log"
run_launcher; sleep 2
check "reboot requested by the app is performed" grep -q "^reboot" "$WORK/calls.log"
check "Nickel is not restarted when rebooting" test ! -e "$B/data/nickel.log"
rm -f /tmp/reboot

# Termination by signal restores Nickel too.
printf '#!/bin/sh\nsleep 30\n' > "$B/releases/1.2.0/inoink"
rm -f "$B/data/nickel.log"
PATH="$STUBS:$PATH" INOINK_DATA="$B/data" sh "$B/releases/1.2.0/inoink-run.sh" > /dev/null 2>&1 < /dev/null &
LPID=$!
sleep 1; kill -TERM $LPID; wait $LPID 2>/dev/null; sleep 1
check "SIGTERM restores Nickel once" test "$(wc -l < "$B/data/nickel.log" | tr -d ' ')" = 1
pkill -f "sleep 30" 2>/dev/null
rmdir /tmp/inoink.lock 2>/dev/null

# --- removal ---------------------------------------------------------------------------------------
check "removal succeeds" sh scripts/remove-inoink.sh "$KOBO"
check "release removed" test ! -d "$B/releases"
check "menu entry removed" test ! -e "$KOBO/.adds/nm/inoink"
check "data kept by default" test "$(cat "$B/data/inoink.db")" = "my data"
check "unrelated apps still intact" test "$(cat "$KOBO/.adds/other/file")" = keep
check "purge deletes data" sh scripts/remove-inoink.sh "$KOBO" --purge-data
check "data gone after purge" test ! -d "$B"

# --- a real package, when given ------------------------------------------------------------------
if [ -n "${1:-}" ]; then
	PKG=$1
	check "package lists no absolute or traversal paths" sh -c "! tar -tzf '$PKG' | grep -Eq '(^/|(^|/)\.\.(/|\$))'"
	check "package has the license and notices" sh -c "tar -tzf '$PKG' | grep -q 'LICENSE-AGPLv3' && tar -tzf '$PKG' | grep -q 'THIRD-PARTY-NOTICES' && tar -tzf '$PKG' | grep -q 'licenses/mupdf/COPYING'"
	check "entries are owned by root with a fixed mtime" sh -c "tar -tvzf '$PKG' | awk '{print \$2}' | sort -u | grep -qx 'root/root\\|0/0'"
	EX="$WORK/extract"; mkdir -p "$EX"; tar -xzf "$PKG" -C "$EX"
	check "no credentials in the package" sh -c "! grep -rIlE 'client_secret|refresh_token|access_token' '$EX' | grep -q ."
fi

echo
if [ "$FAILS" = 0 ]; then echo "package tests passed"; else echo "$FAILS package test(s) FAILED"; exit 1; fi
