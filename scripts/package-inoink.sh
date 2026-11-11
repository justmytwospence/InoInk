#!/bin/sh
# Assembles the installable Kobo package plus matching source archives. Run after
# `build-inoink.sh --target kobo`. Output: dist/inoink-<version>-kobo.tar.gz, -source.tar.gz,
# -third-party-sources.tar and SHA256SUMS. Archives are deterministic (sorted, fixed owner/mtime).
set -eu
cd "$(dirname "$0")/.."

if [ -z "${INOINK_IN_CONTAINER:-}" ]; then
	[ -x target/kobo/arm-unknown-linux-gnueabihf/release/inoink ] || { echo "error: run scripts/build-inoink.sh --target kobo first" >&2; exit 1; }
	exec docker run --rm -v "$PWD":/src -e INOINK_IN_CONTAINER=1 \
		-e SOURCE_DATE_EPOCH="$(git log -1 --format=%ct)" -w /src inoink-kobo-rust:local /src/scripts/package-inoink.sh
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

# Native libraries, each under the soname it declares (what the loader will look for).
L=/opt/kobo/libs
for f in "$L"/*.so; do
	soname=$(arm-linux-gnueabihf-readelf -d "$f" | sed -n 's/.*(SONAME).*\[\(.*\)\]/\1/p')
	[ -n "$soname" ] || soname=$(basename "$f")
	cp "$f" "$R/libs/$soname"
done
# Every needed library must be bundled or part of the Kobo's base system.
SYSTEM=" libc.so.6 libm.so.6 libdl.so.2 libpthread.so.0 librt.so.1 libgcc_s.so.1 libstdc++.so.6 ld-linux-armhf.so.3 "
MISSING=0
for f in "$R/inoink" "$R"/libs/*; do
	for n in $(arm-linux-gnueabihf-readelf -d "$f" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p'); do
		case "$SYSTEM" in *" $n "*) continue ;; esac
		[ -e "$R/libs/$n" ] || { echo "error: $(basename "$f") needs $n, which is not bundled" >&2; MISSING=1; }
	done
done
[ "$MISSING" = 0 ] || exit 1
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
cp LICENSE-MIT "$R/LICENSE-MIT"
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
( cd "$OUT" && sha256sum inoink-* > SHA256SUMS )
cat "$OUT/SHA256SUMS"
