#!/bin/sh
# Native, isolated preview for Apple Silicon Macs: offline fixtures, an offline snapshot of real
# articles (--inoreader), or the live Inoreader account with real sync (--live).
# --capture runs the scripted tour without a window instead and keeps its screens for screen review
# (doc/INOINK-SCREEN-REVIEW.md).
set -eu

usage() {
 echo 'Usage: scripts/preview-macos.sh [--reset] [--inoreader [--refresh] | --live] [--scale NUMBER] [--build-only]'
 echo '       scripts/preview-macos.sh --capture [--inoreader [--refresh]] [--device CODENAME:MODEL]'
}
invalid() { echo "error: $*" >&2; usage >&2; exit 2; }
reset=0 build_only=0 inoreader=0 refresh=0 live=0 scale=0.5 capture=0 device=
while [ "$#" -gt 0 ]; do
 case $1 in
  --help) usage; exit 0 ;;
  --reset) reset=1 ;;
  --capture) capture=1 ;;
  --device) [ "$#" -ge 2 ] || invalid '--device requires CODENAME:MODEL'; device=$2; shift ;;
  --inoreader) inoreader=1 ;;
  --live) live=1 ;;
  --refresh) refresh=1 ;;
  --build-only) build_only=1 ;;
  --scale) [ "$#" -ge 2 ] || invalid '--scale requires a number'; scale=$2; shift ;;
  *) invalid "unknown argument: $1" ;;
 esac
 shift
done
[ "$reset" = 0 ] || [ "$build_only" = 0 ] || invalid '--reset cannot be combined with --build-only'
[ "$refresh" = 0 ] || [ "$inoreader" = 1 ] || invalid '--refresh requires --inoreader'
[ "$refresh" = 0 ] || [ "$build_only" = 0 ] || invalid '--refresh cannot be combined with --build-only'
[ "$reset" = 0 ] || [ "$inoreader" = 0 ] || invalid '--reset cannot be combined with --inoreader; use --refresh'
[ "$live" = 0 ] || [ "$inoreader" = 0 ] || invalid '--live cannot be combined with --inoreader'
[ "$live" = 0 ] || [ "$reset" = 0 ] || invalid '--reset cannot be combined with --live'
[ "$capture" = 0 ] || [ "$live" = 0 ] || invalid '--capture cannot be combined with --live'
[ "$capture" = 0 ] || [ "$build_only" = 0 ] || invalid '--capture cannot be combined with --build-only'
[ "$capture" = 0 ] || [ "$reset" = 0 ] || invalid '--capture always starts from fresh sample data; drop --reset'
[ -z "$device" ] || [ "$capture" = 1 ] || invalid '--device requires --capture'
# The emulator takes the codename and model number from the environment, like a Kobo.
product=${device:-monza:390}; model=${product#*:}; product=${product%:*}
case $product in ''|[!A-Za-z]*|*[!A-Za-z0-9]*) invalid 'invalid device; expected CODENAME:MODEL, for example spaBW:391' ;; esac
case $model in ''|*[!0-9]*) invalid 'invalid device; expected CODENAME:MODEL, for example spaBW:391' ;; esac
# Restrict syntax before numeric comparison, including NaN/Inf and awk expressions.
case $scale in ''|*[!0-9.eE+-]*) invalid 'invalid scale' ;; esac
printf '%s\n' "$scale" | awk 'BEGIN { ok=0 } /^[+-]?([0-9]+([.][0-9]*)?|[.][0-9]+)([eE][+-]?[0-9]+)?$/ { if ($0+0 >= 0.1 && $0+0 <= 2.0) ok=1 } END { exit !ok }' || invalid 'scale must be finite and between 0.1 and 2.0'
[ "$(uname -s)" = Darwin ] && [ "$(uname -m)" = arm64 ] || { echo 'error: native preview requires macOS on Apple Silicon (arm64)' >&2; exit 1; }
for cmd in brew cargo rustc xcrun make curl shasum tar; do
 command -v "$cmd" >/dev/null 2>&1 || { echo "error: install $cmd before running the preview (Xcode command-line tools and Rust are required)" >&2; exit 1; }
done
xcrun --find clang >/dev/null || { echo 'error: install Xcode command-line tools (xcode-select --install)' >&2; exit 1; }
ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd -P)
cd "$ROOT"
BASE=$ROOT/target/macos-preview
NATIVE=$BASE/native
STATE=$BASE/state
LOCK=$BASE/run.lock
mkdir -p "$BASE"
if ! mkdir "$LOCK" 2>/dev/null; then
 pid=$(cat "$LOCK/pid" 2>/dev/null || :)
 case $pid in
  ''|*[!0-9]*) echo 'error: preview lock has no valid PID; refusing automatic removal' >&2 ;;
  *)
   if kill -0 "$pid" 2>/dev/null; then echo "error: preview already running (PID $pid)" >&2; exit 1; fi
   echo "error: stale preview lock (PID $pid); refusing automatic removal" >&2 ;;
 esac
 # A stale PID check cannot safely authorize deletion: another launch may own
 # the path by then. Leave recovery to the user, including incomplete locks.
 echo 'After confirming no preview or build is running, manually remove this directory:' >&2
 printf '  %s\n' "$LOCK" >&2
 exit 1
fi
echo "$$" > "$LOCK/pid"
child= stage= publishing=0 had_data=0 interrupted=0 work=
cleanup() {
 [ -z "$work" ] || rm -rf "$work"
 # A failed publication restores the old data before removing our staging area.
 if [ -n "$stage" ]; then
  if [ "$publishing" = 1 ]; then
   if [ -e "$stage/backup" ]; then
    rm -rf "$STATE/data"
    if ! mv "$stage/backup" "$STATE/data"; then
     echo "error: could not restore snapshot; backup retained at $stage/backup" >&2
     stage=
    fi
   elif [ "$had_data" = 0 ] && [ ! -e "$stage/data" ]; then
    rm -rf "$STATE/data"
   fi
  fi
  [ -z "$stage" ] || rm -rf "$stage"
 fi
 # Only release our own lock; never recursively delete a replacement or its files.
 [ "$(cat "$LOCK/pid" 2>/dev/null || :)" = "$$" ] || return 0
 rm -f "$LOCK/pid"
 rmdir "$LOCK" 2>/dev/null || :
}
on_signal() {
 sig=$1; code=$2
 interrupted=$code
 # The child may have spawned without $! having been assigned yet. Record the
 # cancellation even in that gap; run_child will forward it once the PID is known.
 if [ -n "$child" ]; then kill -s "$sig" "$child" 2>/dev/null || :; fi
}
check_cancelled() { [ "$interrupted" = 0 ] || exit "$interrupted"; }
trap cleanup EXIT
# POSIX asynchronous children inherit SIGINT ignored, even after exec. Translate
# Ctrl-C to TERM so the emulator can shut down, then reap it before releasing lock.
trap 'on_signal TERM 130' INT
trap 'on_signal TERM 143' TERM

formulae='pkgconf jq sdl2 freetype harfbuzz jpeg-turbo libpng openjpeg jbig2dec gumbo-parser djvulibre'
brew_prefix=$(brew --prefix)
missing=
for f in $formulae; do brew list --versions "$f" >/dev/null 2>&1 || missing="$missing $f"; done
if [ -n "$missing" ]; then
 echo "Installing missing Homebrew formulae:$missing"
 # Formula names above are fixed, not user input.
 # shellcheck disable=SC2086
 HOMEBREW_NO_AUTO_UPDATE=1 brew install $missing
else echo 'Homebrew dependencies cached.'; fi
PATH="$(brew --prefix pkgconf)/bin:$(brew --prefix jq)/bin:$PATH"
export PATH
mkdir -p "$NATIVE"
archive_name=mupdf-1.27.0-source.tar.gz
archive=$ROOT/build/inputs/$archive_name
if [ ! -s "$archive" ]; then
 archive=$NATIVE/$archive_name
 if [ ! -s "$archive" ]; then
  echo 'Downloading pinned MuPDF source.'
  curl -fSL --retry 3 -o "$archive.part" 'https://casper.mupdf.com/downloads/archive/mupdf-1.27.0-source.tar.gz'
  mv "$archive.part" "$archive"
 fi
fi
want=$(jq -r --arg n "$archive_name" '.[$n] // empty' "$ROOT/build/inputs.lock.json")
[ -n "$want" ] || { echo 'error: MuPDF checksum missing from lock file' >&2; exit 1; }
got=$(shasum -a 256 "$archive" | cut -d' ' -f1)
[ "$want" = "$got" ] || { echo 'error: MuPDF checksum mismatch' >&2; exit 1; }
# Cargo's encoded flags preserve each argument even when the checkout contains spaces.
sep=$(printf '\037')
flags="-L${sep}native=$NATIVE/lib${sep}-L${sep}native=$ROOT/target/mupdf_wrapper/Darwin"
pcpath=
fingerprint=$NATIVE/fingerprint.tmp
{
 printf '%s\n' "$got" "$(shasum -a 256 "$ROOT/scripts/preview-macos.sh" | cut -d' ' -f1)" "$(rustc -vV | grep '^host:')" "$(xcrun clang --version)" "$brew_prefix"
 for f in $formulae; do
  prefix=$(brew --prefix "$f")
  printf '%s\n' "$f $prefix $(brew list --versions "$f")"
 done
} > "$fingerprint"
for f in $formulae; do
 prefix=$(brew --prefix "$f")
 pcpath=${pcpath:+$pcpath:}$prefix/lib/pkgconfig
 flags="$flags${sep}-L${sep}native=$prefix/lib${sep}-C${sep}link-arg=-Wl,-rpath,$prefix/lib"
done
export PKG_CONFIG_PATH="$pcpath"
if [ ! -f "$NATIVE/stamp" ] || ! cmp -s "$fingerprint" "$NATIVE/stamp" || [ ! -f "$NATIVE/lib/libmupdf.a" ] || [ ! -f "$NATIVE/lib/libmupdf-third.a" ] || [ ! -f "$ROOT/target/mupdf_wrapper/Darwin/libmupdf_wrapper.a" ]; then
 echo 'Building pinned MuPDF and wrapper (first run may take a while).'
 rm -rf "$NATIVE/src" "$NATIVE/lib" "$ROOT/target/mupdf_wrapper/Darwin"
 mkdir -p "$NATIVE/src" "$NATIVE/lib"
 tar -xzf "$archive" -C "$NATIVE/src" --strip-components 1
 jobs=$(sysctl -n hw.ncpu)
 (
  cd "$NATIVE/src"
  export SYS_FREETYPE_CFLAGS="$(pkg-config --cflags freetype2)" SYS_FREETYPE_LIBS="$(pkg-config --libs freetype2)"
  export SYS_HARFBUZZ_CFLAGS="$(pkg-config --cflags harfbuzz)" SYS_HARFBUZZ_LIBS="$(pkg-config --libs harfbuzz)"
  export SYS_LIBJPEG_CFLAGS="$(pkg-config --cflags libjpeg)" SYS_LIBJPEG_LIBS="$(pkg-config --libs libjpeg)"
  export SYS_OPENJPEG_CFLAGS="$(pkg-config --cflags libopenjp2)" SYS_OPENJPEG_LIBS="$(pkg-config --libs libopenjp2)"
  export SYS_JBIG2DEC_CFLAGS="$(pkg-config --cflags jbig2dec)" SYS_JBIG2DEC_LIBS="$(pkg-config --libs jbig2dec)"
  export SYS_GUMBO_CFLAGS="$(pkg-config --cflags gumbo)" SYS_GUMBO_LIBS="$(pkg-config --libs gumbo)"
  export SYS_ZLIB_CFLAGS= SYS_ZLIB_LIBS=-lz
  set -- build=release shared=no HAVE_OBJCOPY=no HAVE_LIBCRYPTO=no USE_SYSTEM_LIBS=yes mujs=no tesseract=no extract=no archive=no brotli=no barcode=no commercial=no 'XCFLAGS=-fPIC -DTOFU_CJK_LANG -DTOFU_CJK_EXT -DFZ_ENABLE_ICC=0 -DFZ_ENABLE_SPOT_RENDERING=0 -DFZ_ENABLE_ODT_OUTPUT=0 -DFZ_ENABLE_OCR_OUTPUT=0'
  make -j"$jobs" "$@" generate
  make -j"$jobs" "$@" libs
 )
 cp "$NATIVE/src/build/release/libmupdf.a" "$NATIVE/src/build/release/libmupdf-third.a" "$NATIVE/lib/"
 # build.sh expands CPPFLAGS unquoted; use a space-free alias for its include path.
 alias_dir=$(mktemp -d /tmp/inoink-preview.XXXXXXXX)
 ln -s "$NATIVE/src/include" "$alias_dir/include"
 if ! (cd "$ROOT/mupdf_wrapper" && CC=clang AR=ar TARGET_OS=Darwin CPPFLAGS="-I$alias_dir/include" CFLAGS='-arch arm64' sh ./build.sh); then rm -rf "$alias_dir"; exit 1; fi
 rm -rf "$alias_dir"
 mv "$fingerprint" "$NATIVE/stamp"
else
 echo 'Native dependencies cached.'
 rm -f "$fingerprint"
fi
version=$(scripts/version.sh dev)
echo 'Building emulator and fixture CLI.'
check_cancelled
CARGO_TARGET_DIR="$BASE/cargo" CARGO_ENCODED_RUSTFLAGS="$flags" INOINK_VERSION="$version" cargo build --locked --target aarch64-apple-darwin -p emulator -p inoink-cli
check_cancelled
[ "$build_only" = 0 ] || exit 0
bin=$BASE/cargo/aarch64-apple-darwin/debug
# Reap children before touching staging or releasing the shared lock. Save stdin
# on entry: dash optimizes <&0 away on an async command, leaving /dev/null there.
# Restore it explicitly in the child, without leaking the saved descriptor.
run_child() {
 check_cancelled
 "$@" <&9 9<&- &
 child=$!
 if [ "$interrupted" != 0 ]; then kill -s TERM "$child" 2>/dev/null || :; fi
 while :; do
  status=0; wait "$child" || status=$?
  if kill -0 "$child" 2>/dev/null; then continue; fi
  break
 done
 child=
 # Cancellation must stop network work before publication. The emulator is
 # always the last operation; retain its established shutdown exit status.
 if [ "$interrupted" != 0 ] && [ "$1" != emulate ]; then return "$interrupted"; fi
 return "$status"
} 9<&0
online=
if [ "$live" = 1 ]; then
 # The real account: changes made here sync to Inoreader, and syncing uses the app
 # registration's daily request allowance (register a separate app to keep the Kobo's apart).
 umask 077
 STATE=$BASE/live
 mkdir -p "$STATE/data"
 echo 'Live preview: changes sync to your Inoreader account.'
 if [ ! -f "$STATE/data/credentials.json" ]; then
  client=${INOINK_CLIENT_ID:-}
  redirect=${INOINK_REDIRECT_URI:-}
  [ -n "$client" ] || { printf 'Inoreader app ID: '; read -r client; }
  [ -n "$redirect" ] || { printf 'Registered redirect URI (for example http://localhost:8080/callback): '; read -r redirect; }
  run_child "$bin/inoink-cli" authorize --client-id "$client" --redirect-uri "$redirect" --out "$STATE/data/credentials.json" || exit $?
 fi
 online=--online
elif [ "$inoreader" = 1 ]; then
 umask 077
 REAL=$BASE/inoreader
 STATE=$REAL/state
 AUTH=$REAL/auth
 mkdir -p "$STATE"
 echo 'Snapshot sandbox: all actions stay local. Refresh replaces local changes.'
 if { [ -e "$STATE/data" ] || [ -L "$STATE/data" ]; } && [ ! -f "$STATE/data/preview-sandbox.json" ]; then
  echo 'error: existing real preview data is missing the sandbox marker' >&2; exit 1
 fi
 if [ ! -e "$STATE/data" ] || [ "$refresh" = 1 ]; then
  if [ ! -f "$AUTH/credentials.json" ]; then
   run_child "$bin/inoink-cli" snapshot-login --auth "$AUTH" || exit $?
  fi
  echo 'Downloading up to 100 unread and 100 saved articles; API quota and publisher requests apply.'
  check_cancelled
  stage=$(mktemp -d "$REAL/staging.XXXXXXXX")
  run_child "$bin/inoink-cli" snapshot --auth "$AUTH" --output "$stage/data" || exit $?
  check_cancelled
  [ -f "$stage/data/preview-sandbox.json" ] || { echo 'error: downloaded snapshot is missing the sandbox marker' >&2; exit 1; }
  had_data=0
  [ ! -e "$STATE/data" ] || had_data=1
  publishing=1
  check_cancelled
  if [ "$had_data" = 1 ]; then mv "$STATE/data" "$stage/backup"; fi
  check_cancelled
  mv "$stage/data" "$STATE/data"
  check_cancelled
  publishing=0
  rm -rf "$stage"
  stage=
  check_cancelled
 fi
elif [ "$capture" = 0 ]; then
 check_cancelled
 [ "$reset" = 0 ] || rm -rf "$STATE"
 mkdir -p "$STATE"
 if [ ! -d "$STATE/data" ]; then
  tmp=$(mktemp -d "$STATE/fixtures.XXXXXXXX")
  if ! "$bin/inoink-cli" fixtures --output "$tmp"; then rm -rf "$tmp"; exit 1; fi
  mv "$tmp" "$STATE/data"
 fi
fi
check_cancelled
if [ "$capture" = 1 ]; then
 # The tour works on a throwaway copy with default settings, so every capture starts from the
 # same state and the preview's own data, settings and screenshots stay as they are.
 mode=sample
 [ "$inoreader" = 0 ] || mode=inoreader
 umask 077
 mkdir -p "$BASE/captures/$mode"
 work=$(mktemp -d "$BASE/captures/.capture.XXXXXXXX")
 if [ "$inoreader" = 1 ]; then
  cp -R "$STATE/data" "$work/data"
 else
  "$bin/inoink-cli" fixtures --output "$work/data" || exit 1
 fi
 check_cancelled
 echo "Capturing the scripted tour on $product ($mode data)."
 capture_tour() {
  unset INOINK_CLIENT_ID INOINK_CLIENT_SECRET INOINK_REDIRECT_URI
  export SDL_VIDEODRIVER=dummy PRODUCT="$product" MODEL_NUMBER="$model" INOINK_SETTINGS="$work/Settings.toml"
  exec "$bin/inoink-emulator" --data "$work/data" --scripted-tour --screenshots "$work/shots"
 }
 run_child capture_tour || exit $?
 check_cancelled
 count=$(find "$work/shots" -maxdepth 1 -name '*.png' 2>/dev/null | wc -l | tr -d ' ')
 [ "$count" -gt 0 ] || { echo 'error: the tour saved no screens' >&2; exit 1; }
 rm -rf "$BASE/captures/$mode/$product"
 mv "$work/shots" "$BASE/captures/$mode/$product"
 echo "Captured $count screens in target/macos-preview/captures/$mode/$product."
 exit 0
fi
mkdir -p "$STATE/screenshots"
echo 'Controls: mouse click/hold/drag; B/F pages; [/] rotate; S screenshot; Escape quits.'
emulate() {
 unset SDL_VIDEODRIVER INOINK_CLIENT_ID INOINK_CLIENT_SECRET INOINK_REDIRECT_URI
 export PRODUCT=monza MODEL_NUMBER=390 INOINK_SETTINGS="$STATE/Settings.toml"
 exec "$bin/inoink-emulator" --data "$STATE/data" --scale "$scale" --screenshots "$STATE/screenshots" $online
}
status=0
run_child emulate || status=$?
exit "$status"
