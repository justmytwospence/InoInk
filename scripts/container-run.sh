#!/bin/sh
# Runs a command inside the desktop development container with the repository mounted.
# Usage: scripts/container-run.sh <command...>
set -e
cd "$(dirname "$0")/.."
IMAGE=${INOINK_DEV_IMAGE:-inoink-dev:local}
if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
	docker build -f build/Containerfile.dev -t "$IMAGE" build
fi
mkdir -p target
exec docker run --rm -v "$PWD":/src -v inoink-cargo:/cargo-cache -e INOINK_UID="$(id -u)" \
	-w /src "$IMAGE" sh -c '
	[ -e thirdparty/mupdf/include ] || ln -sfn /opt/mupdf/include thirdparty/mupdf/include
	[ -e target/mupdf_wrapper/Linux/libmupdf_wrapper.a ] || (cd mupdf_wrapper && ./build.sh)
	exec "$@"' sh "$@"
