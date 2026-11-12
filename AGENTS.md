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
- Dev loop: test a branch on the owner's Kobo before merging. With the Kobo mounted,
  `scripts/kobo-dev.sh` builds the working tree as a dev version (`<last release>-dev.N.<sha>[.dirty]`)
  and installs it (`--ci` installs CI's package for the pushed HEAD instead). Record device evidence
  in the PR (template) and, for acceptance cases, in `doc/INOINK-ACCEPTANCE.md`.
- Releases are automatic: merging to `main` is deploying to everyone. After CI passes on `main`, the
  Release workflow computes the version from the conventional-commit titles since the last `v*` tag
  (`scripts/version.sh next`: `feat` minor, `type!`/`BREAKING CHANGE` major, else patch; doc, CI and
  test-only commits release nothing), tags it and publishes the release. Never tag or bump versions
  by hand; the Cargo.toml versions are a `0.0.0` placeholder. So a PR title is a release decision.

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
