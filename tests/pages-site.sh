#!/bin/sh
# Tests for scripts/pages-site.sh with fake release assets.
set -u
cd "$(dirname "$0")/.."
FAILS=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; FAILS=$((FAILS+1)); }
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/assets" "$WORK/empty"
printf 'package bytes' > "$WORK/assets/inoink-1.2.3-kobo.tar.gz"
if command -v sha256sum >/dev/null 2>&1; then sha=$(sha256sum "$WORK/assets/inoink-1.2.3-kobo.tar.gz" | awk '{ print $1 }'); else sha=$(shasum -a 256 "$WORK/assets/inoink-1.2.3-kobo.tar.gz" | awk '{ print $1 }'); fi

if scripts/pages-site.sh "$WORK/out" "$WORK/assets" >/dev/null \
	&& grep -q '"version":"1.2.3"' "$WORK/out/release.json" \
	&& grep -q "\"size\":13," "$WORK/out/release.json" \
	&& grep -q "\"sha256\":\"$sha\"" "$WORK/out/release.json" \
	&& cmp -s "$WORK/assets/inoink-1.2.3-kobo.tar.gz" "$WORK/out/downloads/inoink-1.2.3-kobo-$sha.tar.gz"; then ok "release.json and the package"; else bad "release.json and the package"; fi
for f in index.html install.sh callback.html relay.js; do [ -f "$WORK/out/$f" ] && ok "$f copied" || bad "$f copied"; done
if scripts/pages-site.sh "$WORK/out2" "$WORK/empty" >/dev/null && [ ! -f "$WORK/out2/release.json" ] && [ -f "$WORK/out2/index.html" ]; then ok "no package: site only"; else bad "no package: site only"; fi
[ "$FAILS" -eq 0 ] || { echo "$FAILS failed"; exit 1; }
echo "All pages-site checks passed."
