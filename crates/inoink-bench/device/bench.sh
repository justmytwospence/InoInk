#!/bin/sh
# NickelMenu's "InoInk benchmark": starts the benchmark in the background, then InoInk as usual.
# The benchmark waits for InoInk's list, runs its scenarios and writes results/*.jsonl here.
# Installed by scripts/kobo-bench.sh as .adds/inoink/bench/bench.sh (doc/INOINK-BENCHMARKS.md).
DIR=$(cd "$(dirname "$0")" && pwd) || exit 1
BASE=$(dirname "$DIR")
[ -f "$BASE/current/inoink.sh" ] || exit 1
mkdir -p "$DIR/results"
chmod +x "$DIR/inkbench" 2>/dev/null
echo "--- $(date) benchmark started" >> "$DIR/results/bench.log"
"$DIR/inkbench" run --root "$BASE" --config "$DIR/bench.cfg" --out "$DIR/results" >> "$DIR/results/bench.log" 2>&1 &
exec "$BASE/current/inoink.sh"
