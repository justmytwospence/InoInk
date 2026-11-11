#!/bin/sh
# Runs every automated check: formatting, library and CLI tests, lints, the native view and
# document tests (in the build container) and the install/launcher tests.
set -eu
cd "$(dirname "$0")/.."
cargo fmt -p inoink -p inoink-cli -- --check
cargo test --locked -p inoink -p inoink-cli
cargo clippy --locked -p inoink -p inoink-cli --all-targets -- -D warnings
scripts/container-run.sh cargo test --locked -p plato-core --tests
sh tests/package.sh "$@"
echo "All checks passed."
