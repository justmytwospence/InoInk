#!/bin/sh
# Assembles the installable Kobo package plus matching source archives. Run after
# `build-inoink.sh --target kobo`. Output: dist/inoink-<version>-kobo.tar.gz, -source.tar.gz,
# -third-party-sources.tar and SHA256SUMS. Archives are deterministic (sorted, fixed owner/mtime).
set -eu
cd "$(dirname "$0")/.."

if [ -z "${INOINK_IN_CONTAINER:-}" ]; then
	[ -x target/kobo/arm-unknown-linux-gnueabihf/release/inoink ] || { echo "error: run scripts/build-inoink.sh --target kobo first" >&2; exit 1; }
	exec docker run --rm --platform linux/amd64 -v "$PWD":/src -e INOINK_IN_CONTAINER=1 \
		-e SOURCE_DATE_EPOCH="$(git log -1 --format=%ct)" -w /src inoink-kobo:local /src/scripts/package-inoink.sh
fi

git config --global --add safe.directory /src
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/inoink/Cargo.toml | head -1)
COMMIT=$(git rev-parse --short=12 HEAD)
EPOCH=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}
OUT=dist
STAGE=$(mktemp -d)
rm -rf "$OUT" && mkdir -p "$OUT"

BIN=target/kobo/arm-unknown-linux-gnueabihf/release/inoink
R="$STAGE/inoink/release"
mkdir -p "$R/libs" "$R/scripts" "$STAGE/inoink/dispatch"
cp "$BIN" "$R/inoink"
arm-linux-gnueabihf-strip "$R/inoink"

# Native libraries under the sonames Plato expects (see dist.sh).
L=/opt/kobo/libs
cp "$L/libz.so" "$R/libs/libz.so.1"
cp "$L/libbz2.so" "$R/libs/libbz2.so.1.0"
cp "$L/libpng16.so" "$R/libs/libpng16.so.16"
cp "$L/libjpeg.so" "$R/libs/libjpeg.so.9"
cp "$L/libopenjp2.so" "$R/libs/libopenjp2.so.7"
cp "$L/libjbig2dec.so" "$R/libs/libjbig2dec.so.0"
cp "$L/libfreetype.so" "$R/libs/libfreetype.so.6"
cp "$L/libharfbuzz.so" "$R/libs/libharfbuzz.so.0"
cp "$L/libgumbo.so" "$R/libs/libgumbo.so.2"
cp "$L/libdjvulibre.so" "$R/libs/libdjvulibre.so.21"
cp "$L/libmupdf.so" "$R/libs/libmupdf.so"
patchelf --remove-rpath "$R"/libs/*
arm-linux-gnueabihf-strip "$R"/libs/*

for d in css icons fonts keyboard-layouts; do cp -R "$d" "$R/$d"; done
find "$R/css" -name '*-user.css' -delete
for s in essid.sh ip.sh resume.sh suspend.sh usb-disable.sh usb-enable.sh wifi-disable.sh wifi-enable.sh; do cp "scripts/$s" "$R/scripts/$s"; done
cp contrib/nickel.sh "$R/nickel.sh"
cp contrib/inoink/inoink-run.sh "$R/inoink-run.sh"
cp contrib/inoink/inoink-dispatch.sh "$STAGE/inoink/dispatch/inoink.sh"
cp contrib/inoink/inoink.nm "$STAGE/inoink/dispatch/inoink.nm"
cp LICENSE-AGPLv3 "$R/LICENSE-AGPLv3"
cp THIRD-PARTY-NOTICES "$R/THIRD-PARTY-NOTICES"
cp -R build/licenses "$R/licenses"
printf '%s\n' "$VERSION" > "$STAGE/inoink/VERSION"
printf 'InoInk %s (%s)\nCorresponding source: inoink-%s-source.tar.gz and inoink-%s-third-party-sources.tar\n' "$VERSION" "$COMMIT" "$VERSION" "$VERSION" > "$R/SOURCE.txt"
chmod +x "$R/inoink" "$R/inoink-run.sh" "$R/nickel.sh" "$R"/scripts/*.sh "$STAGE/inoink/dispatch/inoink.sh"

det_tar() { # dir outfile
	( cd "$1" && find . -mindepth 1 | LC_ALL=C sort | tar --no-recursion --mtime="@$EPOCH" --owner=0 --group=0 --numeric-owner --format=gnu -T - -cf - ) | gzip -n -9 > "$2"
}
det_tar "$STAGE" "$OUT/inoink-$VERSION-kobo.tar.gz"

git archive --format=tar --prefix="inoink-$VERSION/" HEAD | gzip -n -9 > "$OUT/inoink-$VERSION-source.tar.gz"
TP=$(mktemp -d)
cp build/inputs/*.tar.* "$TP/"
cp build/inputs.lock.json "$TP/"
( cd "$TP" && find . -mindepth 1 | LC_ALL=C sort | tar --no-recursion --mtime="@$EPOCH" --owner=0 --group=0 --numeric-owner -T - -cf - ) > "$OUT/inoink-$VERSION-third-party-sources.tar"
( cd "$OUT" && sha256sum ./* | sed 's# \./# #' > SHA256SUMS )
cat "$OUT/SHA256SUMS"
