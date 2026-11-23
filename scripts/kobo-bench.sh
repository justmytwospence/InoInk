#!/bin/sh
# The on-device benchmark (doc/INOINK-BENCHMARKS.md) on a Kobo mounted over USB.
#   kobo-bench.sh build                  cross-build it in the Kobo build container
#   kobo-bench.sh install [mount point]  build it (or fetch CI's build of HEAD without Docker) and
#                                        copy it to .adds/inoink/bench with a NickelMenu entry
#   kobo-bench.sh collect [mount point]  copy the results to target/kobo-bench and summarize them
#   kobo-bench.sh remove [mount point]   remove the benchmark and its menu entry (not the results
#                                        already collected)
#   kobo-bench.sh report FILE...         summarize result files again
# Nothing here touches InoInk's data, settings or releases.
set -eu
cd "$(dirname "$0")/.."

usage() { sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2; }
CMD=${1:-}
[ $# -gt 0 ] && shift

MANIFEST=crates/inoink-bench/Cargo.toml
BIN=target/kobo-bench/arm-unknown-linux-gnueabihf/release/inoink-bench

report() {
	CARGO_TARGET_DIR=target/inoink-bench cargo run -q --locked --manifest-path "$MANIFEST" -- report "$@"
}

# The same pinned toolchain as the app (scripts/build-inoink.sh), in its own target folder.
build() {
	if ! docker image inspect inoink-kobo-rust:local >/dev/null 2>&1; then
		if ! docker image inspect inoink-kobo:local >/dev/null 2>&1; then
			scripts/fetch-inputs.sh
			docker build --platform linux/amd64 -f build/Containerfile -t inoink-kobo:local .
		fi
		docker build -f build/Containerfile.rust -t inoink-kobo-rust:local build
	fi
	docker run --rm -v "$PWD":/src -v inoink-cargo-kobo:/cargo-cache -w /src inoink-kobo-rust:local sh -c "
		export CARGO_TARGET_DIR=/src/target/kobo-bench RUSTFLAGS='-C target-feature=+v7,+vfp3,+neon --remap-path-prefix=/src=. --remap-path-prefix=/cargo-cache=/cargo'
		cargo build --release --locked --manifest-path $MANIFEST --target=arm-unknown-linux-gnueabihf
		arm-linux-gnueabihf-strip $BIN"
}

case $CMD in
report)
	[ $# -gt 0 ] || usage
	report "$@"
	exit ;;
build)
	build
	exit ;;
esac
case $CMD in install|collect|remove) ;; *) usage ;; esac

MNT=${1:-}
if [ -z "$MNT" ]; then
	for m in /Volumes/KOBOeReader /media/"$(id -un)"/KOBOeReader /run/media/"$(id -un)"/KOBOeReader; do
		[ -f "$m/.kobo/version" ] && MNT=$m && break
	done
fi
[ -n "$MNT" ] && [ -f "$MNT/.kobo/version" ] || { echo "error: no Kobo found; connect it by USB and pass its mount point" >&2; exit 1; }
DIR=$MNT/.adds/inoink/bench
OUT=target/kobo-bench

case $CMD in
install)
	[ -f "$MNT/.adds/inoink/current/inoink.sh" ] || { echo "error: InoInk is not installed on this Kobo" >&2; exit 1; }
	[ -d "$MNT/.adds/nm" ] || { echo "error: the benchmark starts from NickelMenu, which is not installed" >&2; exit 1; }
	if docker info >/dev/null 2>&1; then
		build
	else
		echo "Docker is not available; using the benchmark CI built for HEAD." >&2
		SHA=$(git rev-parse HEAD)
		RUN=$(gh run list --workflow CI --commit "$SHA" --status success --json databaseId --jq '.[0].databaseId')
		[ -n "$RUN" ] || { echo "error: no successful CI run for $SHA yet; push it and wait for the kobo check" >&2; exit 1; }
		rm -rf "$OUT/ci" && mkdir -p "$OUT/ci"
		gh run download "$RUN" -n kobo-bench -D "$OUT/ci"
		BIN=$OUT/ci/inoink-bench
	fi
	[ -f "$BIN" ] || { echo "error: $BIN was not built" >&2; exit 1; }
	# Short names only: the Kobo's disk check renames long names created from a computer.
	mkdir -p "$DIR"
	cp "$BIN" "$DIR/inkbench"
	cp crates/inoink-bench/device/bench.sh "$DIR/bench.sh"
	# Keep settings changed on an earlier install.
	[ -f "$DIR/bench.cfg" ] || cp crates/inoink-bench/device/bench.cfg "$DIR/bench.cfg"
	cp crates/inoink-bench/device/inkbench.nm "$MNT/.adds/nm/inkbench"
	sync
	echo "Benchmark installed. Eject the Kobo, then choose InoInk benchmark in NickelMenu and do not touch the screen until the run ends."
	;;
collect)
	[ -d "$DIR/results" ] || { echo "error: no results on this Kobo yet" >&2; exit 1; }
	mkdir -p "$OUT/results"
	new=
	for f in "$DIR"/results/*.jsonl; do
		[ -f "$f" ] || continue
		b=$(basename "$f")
		[ -f "$OUT/results/$b" ] && cmp -s "$f" "$OUT/results/$b" && continue
		cp "$f" "$OUT/results/$b"
		new="$new $OUT/results/$b"
	done
	[ ! -f "$DIR/results/bench.log" ] || cp "$DIR/results/bench.log" "$OUT/results/bench.log"
	[ -n "$new" ] || { echo "No new results (all are in $OUT/results already)."; exit 0; }
	echo "Copied to $OUT/results:$new"
	echo
	# Separately for each file: runs of different releases do not pool.
	for f in $new; do
		echo "## $(basename "$f")"
		report "$f" || :
		echo
	done
	# The requests of the last sync, as InoInk recorded them (Settings > Inoreader API budget).
	DB=$MNT/.adds/inoink/data/inoink.db
	if command -v sqlite3 >/dev/null 2>&1 && [ -f "$DB" ]; then
		n=$(sqlite3 -readonly "$DB" "SELECT value FROM meta WHERE key LIKE '%:last_sync_requests'" 2>/dev/null | head -n 1)
		[ -z "$n" ] || echo "Inoreader requests in the last sync: $n"
	fi
	;;
remove)
	for f in "$DIR"/results/*.jsonl; do
		[ -f "$f" ] || continue
		cmp -s "$f" "$OUT/results/$(basename "$f")" || { echo "error: $(basename "$f") is not collected yet; run kobo-bench.sh collect first" >&2; exit 1; }
	done
	rm -rf "$DIR"
	rm -f "$MNT/.adds/nm/inkbench"
	sync
	echo "Benchmark removed from the Kobo."
	;;
esac
