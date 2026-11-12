# InoInk

Unofficial standalone Inoreader client for the Kobo Libra Colour (firmware 4.46.23852),
derived from [Plato](https://github.com/baskerville/plato) (AGPL-3.0-or-later).

## Policy

- InoInk is a history-preserving derivative of Plato, not a patch stack. The owner explicitly
  chose this over the strict fork-patch-stack convention: ordinary conventional commits on
  `main`, remote `upstream` = Plato, remote `origin` reserved for the owner's InoInk repository.
  There is no legacy Plato library mode. See `doc/UPSTREAM.md`.
- Never force-push `main`. Do not publish or open upstream PRs without asking.
- Never log or commit credentials (tokens, client secret, callback URLs). Secrets are never
  accepted on a command line.
- Hardware claims need hardware evidence. Emulator and automated results are reported separately
  (`doc/INOINK-ACCEPTANCE.md`); unperformed cases stay NOT RUN.

## Workflow

- Every change goes through a pull request from a short bare-slug branch. `main` is protected: the CI
  checks `test`, `core` and `kobo` must pass, history is linear, merging is squash only and the PR
  title is the conventional-commit message. Never push to `main` directly.
- `gh` targets `justmytwospence/InoInk` (`gh repo set-default`). Never open a PR against
  `baskerville/plato`.
- Releases: (1) bump `version` in `crates/inoink/Cargo.toml` and `crates/inoink-cli/Cargo.toml` in a
  PR; (2) after it merges, `git tag vX.Y.Z origin/main && git push origin vX.Y.Z`; (3) the Release
  workflow builds a draft release; (4) record device evidence in `doc/INOINK-ACCEPTANCE.md`, then
  publish the draft.

## Layout

- `crates/inoink`: renderer-independent library (model, SQLite store, Inoreader API client,
  content pipeline, cache, sync worker). No UI dependency; testable without hardware.
- `crates/inoink-cli`: computer-side helper (`authorize`, `provision`, `fixtures`, `verify-cache`).
- `crates/core`, `crates/plato`, `crates/emulator`: inherited renderer, device and app code.

## Commands

- `cargo test --locked -p inoink -p inoink-cli`
- `cargo clippy --locked -p inoink -p inoink-cli --all-targets -- -D warnings`
- `cargo fmt --all -- --check`
- `scripts/test-inoink.sh` runs the three above.
