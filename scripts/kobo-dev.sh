#!/bin/sh
# Builds a dev package of the working tree (uncommitted changes included) and installs it on a
# mounted Kobo, for testing before a pull request is merged. The version names the build, e.g.
# 0.2.3-dev.4.1a2b3c4.dirty (Main menu > Status shows it).
#   kobo-dev.sh [--ci] [mount point] [install-inoink.sh options, e.g. --menu]
#     --ci   skip the local build and install the package CI built for HEAD (HEAD must be
#            pushed and its CI run finished). Used automatically when Docker is not available.
# Installing never touches .adds/inoink/data. Going back to a release: install it the same way
# (scripts/install-inoink.sh), or write the previous release's name into .adds/inoink/active-release.
set -eu
cd "$(dirname "$0")/.."

CI=0
if [ "${1:-}" = "--ci" ]; then
	CI=1
	shift
fi
MNT=${1:-}
[ $# -gt 0 ] && shift
if [ -z "$MNT" ]; then
	for m in /Volumes/KOBOeReader /media/"$(id -un)"/KOBOeReader /run/media/"$(id -un)"/KOBOeReader; do
		[ -f "$m/.kobo/version" ] && MNT=$m && break
	done
fi
[ -n "$MNT" ] && [ -f "$MNT/.kobo/version" ] || { echo "error: no Kobo found; connect it by USB and pass its mount point" >&2; exit 1; }

if [ "$CI" = 0 ] && ! docker info >/dev/null 2>&1; then
	echo "Docker is not available; using the package CI built for HEAD." >&2
	CI=1
fi

if [ "$CI" = 1 ]; then
	[ -z "$(git status --porcelain --untracked-files=no)" ] || { echo "error: uncommitted changes are not in CI's build; commit and push them, or build locally" >&2; exit 1; }
	SHA=$(git rev-parse HEAD)
	RUN=$(gh run list --workflow CI --commit "$SHA" --status success --json databaseId --jq '.[0].databaseId')
	[ -n "$RUN" ] || { echo "error: no successful CI run for $SHA yet; push it and wait for the kobo check" >&2; exit 1; }
	rm -rf dist && mkdir -p dist
	gh run download "$RUN" -n kobo-package -D dist
else
	export INOINK_VERSION
	INOINK_VERSION=$(scripts/version.sh dev)
	echo "Building InoInk $INOINK_VERSION for the Kobo..."
	scripts/build-inoink.sh --target kobo
	scripts/package-inoink.sh
fi

PKG=$(ls dist/inoink-*-kobo.tar.gz)
scripts/install-inoink.sh "$PKG" "$MNT" "$@"
