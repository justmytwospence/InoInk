#! /bin/sh
# Stable entry point: .adds/inoink/current/inoink.sh.
#
# Installs a pending update first: the computer only copies the package as INSTALL.TGZ (plus its
# checksum in INSTALL.SUM) and the Kobo unpacks it here. Files created by the Kobo itself get the
# directory entries its own disk check expects; files and folders created from macOS were renamed
# to FSCK0000.000 by that check, which silently broke updates.
#
# Then picks the active release by name from ../active-release (FAT has no symlinks) and falls
# back to the previous one if it is broken.
BASE=$(cd "$(dirname "$0")/.." && pwd) || exit 1
mkdir -p "$BASE/data" "$BASE/releases"
LOG="$BASE/data/inoink.log"

valid_name() { # letters, digits, dot, dash, underscore; no leading dot
	case "$1" in
		''|.*|*[!A-Za-z0-9._-]*) return 1 ;;
	esac
	return 0
}

valid() { # release name
	valid_name "$1" && [ -f "$BASE/releases/$1/inoink-run.sh" ] && [ -f "$BASE/releases/$1/inoink" ]
}

apply_update() {
	PKG="$BASE/INSTALL.TGZ"
	SUM="$BASE/INSTALL.SUM"
	[ -f "$PKG" ] || return 0
	[ -f "$SUM" ] || return 0   # the computer writes the checksum last: the copy is unfinished
	UNPACK="$BASE/releases/.unpack"
	fail() {
		echo "--- $(date) update not installed: $1" >> "$LOG"
		rm -rf "$UNPACK"
		rm -f "$PKG" "$SUM"
	}
	if [ "$(cksum < "$PKG")" != "$(tr -d '\r\n' < "$SUM")" ]; then fail "the package did not copy correctly"; return 0; fi
	if gzip -dc "$PKG" | tar -tf - 2>/dev/null | grep -Eq '(^/|(^|/)\.\.(/|$))'; then fail "the package contains unsafe paths"; return 0; fi
	rm -rf "$UNPACK"
	mkdir -p "$UNPACK" || { fail "cannot create $UNPACK"; return 0; }
	if ! gzip -dc "$PKG" | tar -xf - -C "$UNPACK"; then fail "the package could not be unpacked"; return 0; fi
	V=$(tr -d '\r\n ' < "$UNPACK/inoink/VERSION" 2>/dev/null)
	valid_name "$V" || { fail "bad version name in the package"; return 0; }
	[ -f "$UNPACK/inoink/release/inoink" ] && [ -f "$UNPACK/inoink/release/inoink-run.sh" ] || { fail "the package is incomplete"; return 0; }
	rm -rf "$BASE/releases/$V"
	mv "$UNPACK/inoink/release" "$BASE/releases/$V" || { fail "cannot move the release into place"; return 0; }
	chmod +x "$BASE/releases/$V/inoink" "$BASE/releases/$V/inoink-run.sh" 2>/dev/null
	CUR=$(cat "$BASE/active-release" 2>/dev/null | tr -d '\r\n ')
	if valid "$CUR" && [ "$CUR" != "$V" ]; then
		printf '%s\n' "$CUR" > "$BASE/previous-release"
	fi
	printf '%s\n' "$V" > "$BASE/active-release"
	# Keep the new release and the previous one; drop older ones and disk-check leftovers.
	PREV=$(cat "$BASE/previous-release" 2>/dev/null | tr -d '\r\n ')
	for d in "$BASE"/releases/* "$BASE"/releases/.unpack; do
		[ -e "$d" ] || continue
		n=$(basename "$d")
		[ "$n" = "$V" ] && continue
		[ -n "$PREV" ] && [ "$n" = "$PREV" ] && continue
		rm -rf "$d"
	done
	rm -f "$PKG" "$SUM"
	sync
	echo "--- $(date) installed release $V" >> "$LOG"
}

apply_update
# For the tests: install a pending update without starting InoInk.
[ -n "${INOINK_UPDATE_ONLY:-}" ] && exit 0

REL=$(cat "$BASE/active-release" 2>/dev/null | tr -d '\r\n ')
if ! valid "$REL"; then
	echo "--- $(date) active release '$REL' is missing or damaged; trying the previous one" >> "$LOG"
	REL=$(cat "$BASE/previous-release" 2>/dev/null | tr -d '\r\n ')
	valid "$REL" || { echo "--- $(date) no valid release installed" >> "$LOG"; echo "InoInk: no valid release installed" >&2; exit 1; }
fi

export INOINK_DATA="$BASE/data"
export INOINK_SETTINGS="$BASE/data/Settings.toml"
exec "$BASE/releases/$REL/inoink-run.sh" "$@"
