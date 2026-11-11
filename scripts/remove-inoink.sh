#!/bin/sh
# Removes InoInk from a mounted Kobo. User data (database, cache, credentials) is kept unless
# --purge-data is given.
#   remove-inoink.sh <mount-point> [--purge-data]
set -eu
MNT=${1:?mount point}
PURGE=0
[ "${2:-}" = "--purge-data" ] && PURGE=1
[ -f "$MNT/.kobo/version" ] || { echo "error: $MNT does not look like a Kobo" >&2; exit 1; }
BASE="$MNT/.adds/inoink"
[ -d "$BASE" ] || { echo "InoInk is not installed."; exit 0; }
rm -rf "$BASE/releases" "$BASE/current" "$BASE/active-release" "$BASE/previous-release"
rm -f "$MNT/.adds/nm/inoink"
if [ "$PURGE" = 1 ]; then
	rm -rf "$BASE"
	echo "InoInk and its data were removed."
else
	echo "InoInk was removed. Your data is kept in $BASE/data (use --purge-data to delete it)."
fi
