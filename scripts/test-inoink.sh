#!/bin/sh
# Runs every automated check: formatting, library and CLI tests, lints, the native view and
# document tests (in the build container), the install/launcher tests and the browser installer's.
set -eu
cd "$(dirname "$0")/.."
sh tests/preview-macos.sh
cargo fmt -p inoink -p inoink-cli -- --check
cargo test --locked -p inoink -p inoink-cli
cargo clippy --locked -p inoink -p inoink-cli --all-targets -- -D warnings
scripts/container-run.sh cargo test --locked -p plato-core --tests
scripts/container-run.sh cargo test --locked -p emulator -- --test-threads=1
sh tests/package.sh "$@"
node --test tests/web-installer.test.mjs
echo "All checks passed."
