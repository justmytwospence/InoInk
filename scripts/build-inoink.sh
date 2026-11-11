#!/bin/sh
# Reproducible builds inside pinned containers.
#   build-inoink.sh --target desktop          build the desktop emulator (Linux container)
#   build-inoink.sh --target kobo             cross-build the Kobo binary (ARM, Linaro GCC 4.9.4)
#   build-inoink.sh --run-emulator --data DIR run the scripted emulator tour on DIR's data and
#                                             write screenshots to target/inoink-emulator-shots
# Nothing here touches an installed application.
set -eu
cd "$(dirname "$0")/.."
TARGET=""
RUN=0
DATA=target/inoink-fixtures
while [ $# -gt 0 ]; do
	case "$1" in
		--target) TARGET=$2; shift ;;
		--run-emulator) RUN=1 ;;
		--data) DATA=$2; shift ;;
		*) echo "unknown option $1" >&2; exit 2 ;;
	esac
	shift
done

kobo_image() {
	if ! docker image inspect inoink-kobo:local >/dev/null 2>&1; then
		scripts/fetch-inputs.sh
		docker build --platform linux/amd64 -f build/Containerfile -t inoink-kobo:local .
	fi
}

kobo_run() {
	kobo_image
	mkdir -p target
	docker run --rm --platform linux/amd64 -v "$PWD":/src -v inoink-cargo-kobo:/cargo-cache \
		-e SOURCE_DATE_EPOCH="$(git log -1 --format=%ct)" -w /src inoink-kobo:local sh -c "$1"
}

case "$TARGET" in
	desktop)
		scripts/container-run.sh cargo build --locked --release -p emulator
		;;
	kobo)
		kobo_run '
			ln -sfn /opt/kobo/thirdparty/mupdf/include thirdparty/mupdf/include
			rm -rf libs && cp -R /opt/kobo/libs libs
			(cd mupdf_wrapper && ./build-kobo.sh)
			export RUSTFLAGS="-C target-feature=+v7,+vfp3,+neon --remap-path-prefix=/src=. --remap-path-prefix=/cargo-cache=/cargo -C link-arg=-Wl,--build-id=none"
			export CARGO_TARGET_DIR=/src/target/kobo
			cargo build --release --locked --target=arm-unknown-linux-gnueabihf -p plato
		'
		;;
	"") ;;
	*) echo "unknown target $TARGET" >&2; exit 2 ;;
esac

if [ "$RUN" = 1 ]; then
	rm -rf target/inoink-emulator-shots
	scripts/container-run.sh sh -c "
		cargo build --locked -p emulator &&
		SDL_VIDEODRIVER=dummy PRODUCT=monza MODEL_NUMBER=390 INOINK_SETTINGS=/tmp/inoink-settings.toml \
		target/linux/debug/inoink-emulator --data '$DATA' --scripted-tour --screenshots target/inoink-emulator-shots"
fi
