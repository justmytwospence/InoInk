#!/bin/sh
# Installs or updates InoInk on a mounted Kobo.
#   install-inoink.sh <package.tar.gz> <mount-point-of-KOBOeReader> [--menu]
# Only .adds/inoink is touched (plus .adds/nm/inoink with --menu when NickelMenu is installed).
# User data (.adds/inoink/data) is never modified. The Kobo installs the release when InoInk next
# starts and keeps the previous one for rollback.
set -eu
PKG=${1:?package}
MNT=${2:?mount point}
MENU=0
[ "${3:-}" = "--menu" ] && MENU=1

# Refuse anything that does not look like a Kobo.
[ -f "$MNT/.kobo/version" ] || { echo "error: $MNT does not look like a Kobo (no .kobo/version)" >&2; exit 1; }
[ -d "$MNT" ] && [ -w "$MNT" ] || { echo "error: $MNT is not writable" >&2; exit 1; }
if [ -d /tmp/inoink.lock ] && [ "$MNT" = "/mnt/onboard" ]; then
	echo "error: InoInk is running; exit it before updating" >&2
	exit 1
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
tar -xzf "$PKG" -C "$TMP"
# Containment: every entry must live under inoink/.
if tar -tzf "$PKG" | grep -Eq '(^/|(^|/)\.\.(/|$))'; then
	echo "error: package contains unsafe paths" >&2
	exit 1
fi
VERSION=$(cat "$TMP/inoink/VERSION")
case "$VERSION" in ''|.*|*[!A-Za-z0-9._-]*) echo "error: bad version in package" >&2; exit 1 ;; esac

BASE="$MNT/.adds/inoink"
mkdir -p "$BASE/current" "$BASE/data"
# The computer only copies two files with plain 8.3 names; the Kobo unpacks the release itself the
# next time InoInk starts (see contrib/inoink/inoink-dispatch.sh). Folders and long names created
# from macOS were renamed to FSCK0000.000 by the Kobo's disk check, which broke updates.
rm -f "$BASE/INSTALL.SUM"
cp "$PKG" "$BASE/INSTALL.TGZ"
sync
[ "$(cksum < "$PKG")" = "$(cksum < "$BASE/INSTALL.TGZ")" ] || { rm -f "$BASE/INSTALL.TGZ"; echo "error: the copy to the Kobo is damaged; nothing was changed" >&2; exit 1; }
cp "$TMP/inoink/dispatch/inoink.sh" "$BASE/current/inoink.sh"
chmod +x "$BASE/current/inoink.sh" 2>/dev/null || true
# The checksum goes last: the Kobo ignores a package without one.
cksum < "$PKG" > "$BASE/INSTALL.SUM"
sync

if [ "$MENU" = 1 ]; then
	if [ -d "$MNT/.adds/nm" ]; then
		cp "$TMP/inoink/dispatch/inoink.nm" "$MNT/.adds/nm/inoink"
	else
		echo "NickelMenu is not installed; no launcher entry was added." >&2
	fi
fi
sync
if [ "${INOINK_NO_EJECT:-}" != 1 ] && command -v diskutil >/dev/null 2>&1; then
	diskutil eject "$MNT" >/dev/null && echo "InoInk $VERSION copied and the Kobo ejected; it installs when you next open InoInk." && exit 0
	echo "InoInk $VERSION copied, but the Kobo could not be ejected; eject it before unplugging." >&2
	exit 0
fi
echo "InoInk $VERSION copied; it installs when you next open InoInk. Eject the Kobo before unplugging it."
