#!/bin/sh
# Tests for site/install.sh with a fake release, a fake Kobo and stub installer and helper.
#   tests/site-install.sh
set -u
cd "$(dirname "$0")/.."
ROOT=$PWD
FAILS=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; FAILS=$((FAILS+1)); }

case "$(uname -s) $(uname -m)" in
"Darwin arm64") PLAT=macos-arm64 ;;
"Linux x86_64") PLAT=linux-x86_64 ;;
*) echo "ok   skipped (no inoink-cli for this platform)"; exit 0 ;;
esac

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
DL=$WORK/dl/v9.9.9
mkdir -p "$DL" "$WORK/api/releases" "$WORK/kobo/.kobo"
echo x > "$WORK/kobo/.kobo/version"
printf '#!/bin/sh\necho "install $*" >> "%s/log"\n' "$WORK" > "$DL/install-inoink.sh"
printf '#!/bin/sh\necho "cli $*" >> "%s/log"\n' "$WORK" > "$DL/inoink-cli-$PLAT"
echo package > "$DL/inoink-9.9.9-kobo.tar.gz"
(cd "$DL" && if command -v sha256sum >/dev/null 2>&1; then sha256sum *; else shasum -a 256 *; fi > SHA256SUMS)
echo '{"tag_name": "v9.9.9"}' > "$WORK/api/releases/latest"

export INOINK_API="file://$WORK/api" INOINK_DOWNLOADS="file://$WORK/dl" INOINK_TTY=/dev/null INOINK_NO_EJECT=1
run() { : > "$WORK/log"; sh site/install.sh "$@" > "$WORK/out" 2>&1; }

if run --device "$WORK/kobo" \
	&& grep -q "^install .*/inoink-9.9.9-kobo.tar.gz $WORK/kobo --menu$" "$WORK/log" \
	&& grep -q "^cli setup --device $WORK/kobo$" "$WORK/log"; then ok "installs, then signs in"; else bad "installs, then signs in"; fi

if run --device "$WORK/kobo" --accounts-only --service karakeep \
	&& [ "$(cat "$WORK/log")" = "cli setup --device $WORK/kobo --service karakeep" ]; then ok "accounts only"; else bad "accounts only"; fi

if (INOINK_API="file://$WORK/missing"; export INOINK_API; run --device "$WORK/kobo" --version 9.9.9) \
	&& grep -q "^cli setup" "$WORK/log"; then ok "a named version skips the API"; else bad "a named version skips the API"; fi

cp "$DL/SHA256SUMS" "$WORK/sums.bak"
echo tampered >> "$DL/inoink-9.9.9-kobo.tar.gz"
if run --device "$WORK/kobo"; then bad "tampered package"; elif [ ! -s "$WORK/log" ] && grep -q "does not match" "$WORK/out"; then ok "tampered package is refused"; else bad "tampered package is refused"; fi
echo package > "$DL/inoink-9.9.9-kobo.tar.gz"

if run --device "$WORK/nokobo"; then bad "missing Kobo"; elif grep -q "does not look like a Kobo" "$WORK/out"; then ok "a folder that is not a Kobo is refused"; else bad "a folder that is not a Kobo is refused"; fi

if run --device "$WORK/kobo" --service 'a;b'; then bad "bad service name"; else ok "a bad service name is refused"; fi

if [ "$FAILS" -gt 0 ]; then echo "$FAILS failed"; exit 1; fi
echo "All site install checks passed."
