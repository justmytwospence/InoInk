#!/bin/sh
# Installs or updates InoInk on a mounted Kobo.
#   install-inoink.sh <package.tar.gz> <mount-point-of-KOBOeReader> [--menu]
# Only .adds/inoink is touched (plus .adds/nm/inoink with --menu when NickelMenu is installed).
# User data (.adds/inoink/data) is never modified; the previous release is kept for rollback.
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
mkdir -p "$BASE/releases" "$BASE/current" "$BASE/data"
STAGE="$BASE/releases/.stage-$VERSION"
rm -rf "$STAGE"
cp -R "$TMP/inoink/release" "$STAGE"
chmod +x "$STAGE/inoink" "$STAGE/inoink-run.sh"
sync
# Verify every copied file before activating the release.
( cd "$TMP/inoink/release" && find . -type f | LC_ALL=C sort | while IFS= read -r f; do
	a=$(cksum < "$f"); b=$(cksum < "$STAGE/$f" 2>/dev/null || echo missing)
	[ "$a" = "$b" ] || { echo "error: $f did not copy correctly" >&2; exit 1; }
done ) || { rm -rf "$STAGE"; echo "error: the copy to the Kobo is damaged; nothing was activated" >&2; exit 1; }
[ "$(cd "$TMP/inoink/release" && find . -type f | wc -l)" = "$(cd "$STAGE" && find . -type f | wc -l)" ] || { rm -rf "$STAGE"; echo "error: unexpected files in the copy; nothing was activated" >&2; exit 1; }
rm -rf "$BASE/releases/$VERSION"
mv "$STAGE" "$BASE/releases/$VERSION"
sync

# Keep the previous release for rollback, then activate by replacing the small manifest.
if [ -f "$BASE/active-release" ]; then
	PREV=$(tr -d '\r\n ' < "$BASE/active-release")
	[ "$PREV" != "$VERSION" ] && printf '%s\n' "$PREV" > "$BASE/previous-release"
fi
cp "$TMP/inoink/dispatch/inoink.sh" "$BASE/current/inoink.sh.new"
chmod +x "$BASE/current/inoink.sh.new"
mv "$BASE/current/inoink.sh.new" "$BASE/current/inoink.sh"
printf '%s\n' "$VERSION" > "$BASE/active-release.new"
mv "$BASE/active-release.new" "$BASE/active-release"
sync

# Drop releases older than the previous one.
KEEP="$VERSION $(cat "$BASE/previous-release" 2>/dev/null | tr -d '\r\n ' || true)"
for d in "$BASE"/releases/*; do
	n=$(basename "$d")
	case " $KEEP " in *" $n "*) ;; *) rm -rf "$d" ;; esac
done

if [ "$MENU" = 1 ]; then
	if [ -d "$MNT/.adds/nm" ]; then
		cp "$TMP/inoink/dispatch/inoink.nm" "$MNT/.adds/nm/inoink"
	else
		echo "NickelMenu is not installed; no launcher entry was added." >&2
	fi
fi
sync
if [ "${INOINK_NO_EJECT:-}" != 1 ] && command -v diskutil >/dev/null 2>&1; then
	diskutil eject "$MNT" >/dev/null && echo "InoInk $VERSION installed and the Kobo ejected; you can unplug it." && exit 0
	echo "InoInk $VERSION installed, but the Kobo could not be ejected; eject it before unplugging." >&2
	exit 0
fi
echo "InoInk $VERSION installed. Eject the Kobo before unplugging it."
