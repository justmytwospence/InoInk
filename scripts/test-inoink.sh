#!/bin/sh
# Runs every automated check: formatting, library and CLI tests, lints, the development tools'
# tests (device benchmark, screen review), the native view and document tests (in the build
# container), the install/launcher tests and the browser installer's.
set -eu
cd "$(dirname "$0")/.."
sh tests/preview-macos.sh
cargo fmt -p inoink -p inoink-cli -- --check
cargo test --locked -p inoink -p inoink-cli
cargo clippy --locked -p inoink -p inoink-cli --all-targets -- -D warnings
# Development tools: the device benchmark (its own workspace) and screen review.
M=crates/inoink-bench/Cargo.toml
cargo fmt --manifest-path "$M" -- --check
CARGO_TARGET_DIR=target/inoink-bench cargo test --locked --manifest-path "$M"
CARGO_TARGET_DIR=target/inoink-bench cargo clippy --locked --manifest-path "$M" --all-targets -- -D warnings
sh tests/kobo-bench.sh
node --test scripts/review/review.test.mjs
scripts/container-run.sh cargo test --locked -p plato-core --tests
scripts/container-run.sh cargo test --locked -p emulator -- --test-threads=1
sh tests/package.sh "$@"
node --test tests/web-installer.test.mjs
echo "All checks passed."
