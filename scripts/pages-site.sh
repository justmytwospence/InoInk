#!/bin/sh
# Builds the project site for Pages: site/ plus the latest release's Kobo package under downloads/
# (its checksum in the name) and release.json, so the browser installer can download the package
# from the same site (GitHub's release files send no CORS headers).
#   scripts/pages-site.sh OUT [ASSETS_DIR]     (without ASSETS_DIR: `gh release download`)
set -eu
cd "$(dirname "$0")/.."
OUT=${1:?output directory}
rm -rf "$OUT"
cp -R site "$OUT"
none() { echo "No release package; the page will ask GitHub."; exit 0; }
if [ -n "${2:-}" ]; then
	ASSETS=$2
else
	ASSETS=$(mktemp -d)
	gh release download --repo justmytwospence/InoInk --pattern 'inoink-*-kobo.tar.gz' --dir "$ASSETS" >/dev/null 2>&1 || none
fi
pkg=$(ls "$ASSETS"/inoink-*-kobo.tar.gz 2>/dev/null | head -n 1 || true)
[ -n "$pkg" ] || none
name=$(basename "$pkg")
v=${name#inoink-}
v=${v%-kobo.tar.gz}
case $v in '' | *[!0-9A-Za-z._-]*) echo "bad version $v" >&2; exit 1 ;; esac
if command -v sha256sum >/dev/null 2>&1; then sha=$(sha256sum "$pkg" | awk '{ print $1 }'); else sha=$(shasum -a 256 "$pkg" | awk '{ print $1 }'); fi
size=$(wc -c < "$pkg" | tr -d ' ')
mkdir -p "$OUT/downloads"
cp "$pkg" "$OUT/downloads/inoink-$v-kobo-$sha.tar.gz"
printf '{"version":"%s","name":"%s","file":"downloads/inoink-%s-kobo-%s.tar.gz","size":%s,"sha256":"%s"}\n' "$v" "$name" "$v" "$sha" "$size" "$sha" > "$OUT/release.json"
echo "Site built with InoInk $v ($size bytes)."
