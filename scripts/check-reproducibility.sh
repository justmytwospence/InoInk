#!/bin/sh
# Builds and packages twice from clean output directories and compares the artifact hashes.
# Reports differences instead of claiming reproducibility from a single build.
set -eu
cd "$(dirname "$0")/.."
run() {
	rm -rf target/kobo dist libs
	scripts/build-inoink.sh --target kobo
	scripts/package-inoink.sh
	cp dist/SHA256SUMS "$1"
}
run /tmp/inoink-build-1.sums
run /tmp/inoink-build-2.sums
if diff -u /tmp/inoink-build-1.sums /tmp/inoink-build-2.sums; then
	echo "Reproducible: both builds produced identical artifacts."
else
	echo "NOT reproducible: the artifacts above differ." >&2
	exit 1
fi
