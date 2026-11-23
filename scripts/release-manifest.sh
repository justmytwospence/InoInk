#!/bin/sh
# Writes DIR/MANIFEST, one "checksum size path" line (POSIX cksum) per file of a release. The Kobo
# checks the checksums when it installs the release and the sizes every time InoInk starts (see
# contrib/inoink/inoink-dispatch.sh).
#   release-manifest.sh DIR
set -eu
DIR=${1:?release directory}
cd "$DIR"
rm -f MANIFEST MANIFEST.tmp
LIST=$(find . -type f | sed 's|^\./||' | LC_ALL=C sort)
if printf '%s\n' "$LIST" | grep -q '[[:space:]]'; then
	echo "error: a file name in the release contains a space; the Kobo's check cannot read it" >&2
	exit 1
fi
printf '%s\n' "$LIST" | xargs cksum > MANIFEST.tmp
mv MANIFEST.tmp MANIFEST
