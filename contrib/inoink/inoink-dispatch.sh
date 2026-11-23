#! /bin/sh
# Stable entry point: .adds/inoink/current/inoink.sh.
#   inoink.sh          install a pending update, check the release, start InoInk
#   inoink.sh --check  check only, while the Kobo interface still runs: exits 1 when InoInk cannot
#                      start (the NickelMenu entry then runs --why instead of starting it)
#   inoink.sh --why    print, for NickelMenu to show, why InoInk cannot start
#
# Installs a pending update first: the computer only copies the package as INSTALL.TGZ (plus its
# checksum in INSTALL.SUM) and the Kobo unpacks it here. Files created by the Kobo itself get the
# directory entries its own disk check expects; files and folders created from macOS were renamed
# to FSCK0000.000 by that check, which silently broke updates.
#
# Then picks the active release by name from ../active-release (FAT has no symlinks) and checks it
# before the Kobo interface stops: every file its MANIFEST lists must be there with its size (the
# checksums are checked once, at install), and it must not have failed to load before (LOADFAIL,
# written by inoink-run.sh). A damaged release falls back to the previous one, with a notice; with
# none usable the Kobo interface keeps running and --why explains.
BASE=$(cd "$(dirname "$0")/.." && pwd) || exit 1
MODE=${1:-}
mkdir -p "$BASE/data" "$BASE/releases"
LOG="$BASE/data/inoink.log"
log() { [ "$MODE" = "" ] && echo "--- $(date) $1" >> "$LOG"; return 0; }

valid_name() { # letters, digits, dot, dash, underscore; no leading dot
	case "$1" in
		''|.*|*[!A-Za-z0-9._-]*) return 1 ;;
	esac
	return 0
}

valid() { # release name
	valid_name "$1" && [ -f "$BASE/releases/$1/inoink-run.sh" ] && [ -f "$BASE/releases/$1/inoink" ]
}

# check_files DIR size|crc: prints each file DIR/MANIFEST lists that is missing or differs (in its
# size, or also its checksum) and fails if there is one. Releases without a manifest pass.
check_files() {
	[ -f "$1/MANIFEST" ] || return 0
	if [ "$2" = crc ]; then
		( cd "$1" && awk '{ print $3 }' MANIFEST | xargs cksum 2>/dev/null )
	else
		( cd "$1" && awk '{ print $3 }' MANIFEST | xargs ls -l 2>/dev/null | awk '{ print "-", $5, $NF }' )
	fi | awk -v m="$1/MANIFEST" -v mode="$2" '
		BEGIN { while ((getline l < m) > 0) { if (split(l, f, " ") >= 3) { want[f[3]] = (mode == "crc" ? f[1] " " : "") f[2] } } }
		{ got = (mode == "crc" ? $1 " " : "") $2; if (($3 in want) && want[$3] == got) seen[$3] = 1 }
		END { bad = 0; for (p in want) if (!(p in seen)) { print p; bad = 1 } exit bad }'
}

# usable NAME: the release can start. Prints what is wrong otherwise (at most three files).
usable() {
	valid "$1" || { echo "its program is missing"; return 1; }
	if [ -f "$BASE/releases/$1/LOADFAIL" ]; then echo "it could not be loaded on this Kobo"; return 1; fi
	BAD=$(check_files "$BASE/releases/$1" size) && return 0
	printf 'some of its files are missing or damaged (%s)\n' "$(printf '%s\n' "$BAD" | head -n 3 | tr '\n' ' ' | sed 's/ $//')"
	return 1
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
	# What was written to the Kobo's storage must match what was packaged.
	BAD=$(check_files "$UNPACK/inoink/release" crc) || { fail "unpacked files differ from the package: $(printf '%s\n' "$BAD" | head -n 3 | tr '\n' ' ')"; return 0; }
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

# pick: sets REL to the release to start (and NOTICE when it is not the active one), or fails
# with WHY saying what is wrong with the active release.
pick() {
	ACT=$(cat "$BASE/active-release" 2>/dev/null | tr -d '\r\n ')
	NOTICE=
	WHY=$(usable "$ACT") && { REL=$ACT; return 0; }
	log "active release '$ACT' cannot start: $WHY; trying the previous one"
	PREV=$(cat "$BASE/previous-release" 2>/dev/null | tr -d '\r\n ')
	if [ "$PREV" != "$ACT" ] && usable "$PREV" > /dev/null; then
		REL=$PREV
		log "starting the previous release $PREV"
		NOTICE="InoInk $ACT is damaged, so $PREV started. Reinstall InoInk."
		return 0
	fi
	log "no release can start"
	return 1
}

case "$MODE" in
	--check)
		# A pending update is installed (and checked) by the real start.
		[ -f "$BASE/INSTALL.TGZ" ] && [ -f "$BASE/INSTALL.SUM" ] && exit 0
		pick
		exit $?
		;;
	--why)
		if pick; then
			echo "InoInk is ready to start. Open it again."
		elif [ -z "$(ls "$BASE/releases" 2>/dev/null)" ]; then
			echo "InoInk can't start: it is not installed yet. Install it from your computer (install-inoink.sh), then open it again."
		else
			echo "InoInk can't start: ${WHY:-its files are damaged}."
			echo
			echo "Reinstall it from your computer: install-inoink.sh with the package and this Kobo. Your articles, sign-in and settings are kept."
			case "$WHY" in *loaded*) echo; echo "If this keeps happening, this Kobo may not be supported yet: please open a Device report on github.com/justmytwospence/InoInk." ;; esac
			echo
			echo "Details: .adds/inoink/data/inoink.log"
		fi
		exit 0
		;;
	'') ;;
	*) echo "usage: inoink.sh [--check|--why]" >&2; exit 2 ;;
esac

apply_update
# For the tests: install a pending update without starting InoInk.
[ -n "${INOINK_UPDATE_ONLY:-}" ] && exit 0

if ! pick; then
	# The Kobo interface was never stopped: nothing to restore.
	echo "InoInk can't start: $WHY. Reinstall it." >&2
	exit 1
fi

export INOINK_DATA="$BASE/data"
export INOINK_SETTINGS="$BASE/data/Settings.toml"
export INOINK_NOTICE="$NOTICE"
exec "$BASE/releases/$REL/inoink-run.sh"
