#!/bin/sh
# Removes InoInk from a mounted Kobo. User data (database, cache, sign-in, settings) is kept
# unless asked otherwise. The project page's browser installer removes the same files.
#   remove-inoink.sh <mount-point> [--reset-settings | --purge-data]
#   --reset-settings  also deletes the settings (data/Settings.toml and data/config.sh)
#   --purge-data      deletes .adds/inoink completely, data included
set -eu
MNT=${1:?mount point}
MODE=keep
case "${2:-}" in
	'') ;;
	--reset-settings) MODE=settings ;;
	--purge-data) MODE=all ;;
	*) echo "usage: $0 <mount-point> [--reset-settings | --purge-data]" >&2; exit 2 ;;
esac
[ -f "$MNT/.kobo/version" ] || { echo "error: $MNT does not look like a Kobo" >&2; exit 1; }
BASE="$MNT/.adds/inoink"
rm -f "$MNT/.adds/nm/inoink"
[ -d "$BASE" ] || { echo "InoInk is not installed."; exit 0; }
rm -rf "$BASE/releases" "$BASE/current" "$BASE/active-release" "$BASE/previous-release" \
	"$BASE/INSTALL.TGZ" "$BASE/INSTALL.SUM"
case "$MODE" in
	all)
		rm -rf "$BASE"
		echo "InoInk and its data were removed."
		;;
	settings)
		rm -f "$BASE/data/Settings.toml" "$BASE/data/config.sh"
		echo "InoInk was removed and its settings reset. Your sign-in, articles and highlights are kept in $BASE/data."
		;;
	*)
		echo "InoInk was removed. Your data is kept in $BASE/data (use --purge-data to delete it)."
		;;
esac
