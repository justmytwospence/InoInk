#! /bin/sh
# Stable entry point: .adds/inoink/current/inoink.sh. Picks the active release by name from
# ../active-release (FAT has no symlinks) and falls back to the previous one if it is broken.
BASE=$(cd "$(dirname "$0")/.." && pwd) || exit 1

valid() { # release name: letters, digits, dot, dash, underscore; no leading dot
	case "$1" in
		''|.*|*[!A-Za-z0-9._-]*) return 1 ;;
	esac
	[ -x "$BASE/releases/$1/inoink-run.sh" ] && [ -x "$BASE/releases/$1/inoink" ]
}

REL=$(cat "$BASE/active-release" 2>/dev/null | tr -d '\r\n ')
if ! valid "$REL"; then
	REL=$(cat "$BASE/previous-release" 2>/dev/null | tr -d '\r\n ')
	valid "$REL" || { echo "--- $(date) no valid release installed" >> "$BASE/data/inoink.log"; echo "InoInk: no valid release installed" >&2; exit 1; }
fi

export INOINK_DATA="$BASE/data"
export INOINK_SETTINGS="$BASE/data/Settings.toml"
exec "$BASE/releases/$REL/inoink-run.sh" "$@"
