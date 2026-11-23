#!/bin/sh
# scripts/kobo-bench.sh against a pretend Kobo: what it copies, keeps and removes. No Docker,
# network or device; docker, gh and cargo are stand-ins.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd -P)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
repo="$WORK/repo"; bin="$WORK/bin"; kobo="$WORK/KOBOeReader"; log="$WORK/log"
mkdir -p "$repo/scripts" "$repo/crates/inoink-bench/device" "$bin" "$kobo/.kobo" "$kobo/.adds/inoink/current" "$kobo/.adds/inoink/data"
cp "$ROOT/scripts/kobo-bench.sh" "$repo/scripts/"
cp "$ROOT"/crates/inoink-bench/device/* "$repo/crates/inoink-bench/device/"
echo 'N000,4.1,4.46.23852,4.1,4.1,00000000-0000-0000-0000-000000000390' > "$kobo/.kobo/version"
echo 'app' > "$kobo/.adds/inoink/current/inoink.sh"
echo 'my data' > "$kobo/.adds/inoink/data/keep"
export TEST_LOG="$log" PATH="$bin:$PATH"
cat > "$bin/docker" <<'EOF'
#!/bin/sh
echo "docker $*" >> "$TEST_LOG"
case $1 in
 info) [ "${NO_DOCKER:-0}" = 0 ] ;;
 run) mkdir -p target/kobo-bench/arm-unknown-linux-gnueabihf/release; echo arm-binary > target/kobo-bench/arm-unknown-linux-gnueabihf/release/inoink-bench ;;
esac
EOF
cat > "$bin/gh" <<'EOF'
#!/bin/sh
echo "gh $*" >> "$TEST_LOG"
case $1 in
 run) if [ "$2" = list ]; then echo 42; else d=; while [ $# -gt 0 ]; do [ "$1" = -D ] && d=$2; shift; done; echo ci-binary > "$d/inoink-bench"; fi ;;
esac
EOF
cat > "$bin/git" <<'EOF'
#!/bin/sh
echo 0123456789abcdef
EOF
cat > "$bin/cargo" <<'EOF'
#!/bin/sh
echo "cargo $*" >> "$TEST_LOG"
shift $(($# - 1)); echo "summary of $(basename "$1")"
EOF
chmod +x "$bin"/*
run() { (cd "$repo" && sh scripts/kobo-bench.sh "$@") > "$WORK/out" 2>&1; }
fail() { echo "FAIL: $*"; cat "$WORK/out"; exit 1; }

run && fail 'no command accepted' || :
run install "$WORK/nowhere" && fail 'not a Kobo accepted' || :
run install "$kobo" && fail 'install without NickelMenu accepted' || :
mkdir -p "$kobo/.adds/nm"
run install "$kobo" || fail install
d="$kobo/.adds/inoink/bench"
[ "$(cat "$d/inkbench")" = arm-binary ] || fail 'binary not copied'
cmp -s "$d/bench.sh" "$ROOT/crates/inoink-bench/device/bench.sh" && cmp -s "$d/bench.cfg" "$ROOT/crates/inoink-bench/device/bench.cfg" || fail 'device files'
cmp -s "$kobo/.adds/nm/inkbench" "$ROOT/crates/inoink-bench/device/inkbench.nm" || fail 'menu entry'
# Only short names on the Kobo: its disk check renames long ones made by a computer.
for f in "$d"/* "$kobo/.adds/nm/inkbench"; do
 b=$(basename "$f"); n=${b%%.*}; e=${b#"$n"}; e=${e#.}
 [ ${#n} -le 8 ] && [ ${#e} -le 3 ] || fail "long name on the Kobo: $b"
done
echo 'tap_sync = 1,2' > "$d/bench.cfg"
NO_DOCKER=1 run install "$kobo" || fail 'install from CI'
[ "$(cat "$d/inkbench")" = ci-binary ] || fail 'CI binary not used without Docker'
[ "$(cat "$d/bench.cfg")" = 'tap_sync = 1,2' ] || fail 'install replaced edited settings'

run collect "$kobo" && fail 'collect without results accepted' || :
mkdir -p "$d/results"
echo '{"type":"header"}' > "$d/results/20261120T100000Z-0.9.1.jsonl"
echo 'log' > "$d/results/bench.log"
run remove "$kobo" && fail 'remove before collect accepted' || :
[ -d "$d" ] || fail 'remove deleted uncollected results'
run collect "$kobo" || fail collect
[ -f "$repo/target/kobo-bench/results/20261120T100000Z-0.9.1.jsonl" ] && [ -f "$repo/target/kobo-bench/results/bench.log" ] || fail 'results not copied'
grep -q 'summary of 20261120T100000Z-0.9.1.jsonl' "$WORK/out" || fail 'no summary'
run collect "$kobo" || fail 'second collect'
grep -q 'No new results' "$WORK/out" || fail 'collected twice'
run remove "$kobo" || fail remove
[ ! -e "$d" ] && [ ! -e "$kobo/.adds/nm/inkbench" ] || fail 'remove left files'
[ "$(cat "$kobo/.adds/inoink/data/keep")" = 'my data' ] && [ -f "$kobo/.adds/inoink/current/inoink.sh" ] || fail 'touched InoInk'
echo 'kobo-bench tests passed'
