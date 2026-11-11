# Acceptance evidence

Three kinds of evidence are kept apart. **Automated** means unit, integration and fault tests run
on the development computer with a simulated Inoreader. **Emulator** means the desktop build under
SDL's dummy video driver. **Device** means a physical Kobo Libra Colour. Emulator results are never
evidence about hardware.

Status words: PASS (done, evidence named), NOT RUN (not performed), N/A.

## Automated

Run with `scripts/test-inoink.sh` (format check, library and CLI tests, lints, native view and
document tests inside the build container, and the install/launcher tests).

| Area | Where | Result |
|---|---|---|
| Local store: durable intent, Done vs Mark read, undo before/after acknowledgement, toggles, generation racing acknowledgement, independent tags, frozen bulk scope, stale snapshots, reconciliation, migrations, disk full, accounts | `crates/inoink/tests/store_intent.rs` (22) | PASS |
| API: every supported request shape, IDs (signed/hex), pagination parameters, encoding, Active Search incl. HTTP 202 and destructive GET, token refresh once, revocation, malformed/oversized responses, quota zones, reserve, 429 and persistence | `crates/inoink/tests/api_contract.rs` (23) | PASS |
| Sync faults: first sync incl. old starred, old star elsewhere, remote unstar, Done offline then restart, lost response, partial failure, crash in flight, toggles, tags, pending-wins-then-remote-wins, partial/failed/looping/capped listings, second-listing confirmation, silent unread boundary, token rotation/revocation, quota pause and resume, zone independence, interval, organization changes incl. lost responses, storage pressure, history paging | `crates/inoink/tests/sync_faults.rs` (28) | PASS |
| Content and cache: lazy/responsive/relative images, order and captions, sanitizing, malformed markup, tables/code, normalization and limits, decode bombs, bad MIME, SSRF and redirects, retries, concurrency, cancellation, leases, recovery of interrupted writes, symlinks, rooted paths, eviction and protection, plan priorities, low space, extraction | `crates/inoink/tests/content_cache.rs` (21) | PASS |
| Lifecycle: UI reads/writes stay fast during slow sync, suspend cancels and quiesces, hung network does not hold suspend, no timers when idle, flapping network coalesced, debounce, USB closes the database fully and reopens, pending work survives shutdown, browse/open online | `crates/inoink/tests/lifecycle.rs` (9) | PASS |
| Documents and views: rooted resources (incl. symlinks), image hit data, offsets under font/rotation change, defaults, Done/Undo/Mark read, opening does not mark read, failed save, frozen list scope, tags | `crates/core/tests/inoink_document.rs` (3), `inoink_views.rs` (7), in the container | PASS |
| Install, update, rollback, removal, launcher (Nickel restored once on exit, failure, SIGTERM; double launch; reboot intent), package safety | `tests/package.sh` | PASS |
| Inherited Plato unit tests | `crates/core` | PASS (29) |

## Build and package (computer only)

| Check | Result |
|---|---|
| `scripts/build-inoink.sh --target kobo` cross-builds the ARM binary with the pinned Linaro GCC 4.9.4 toolchain (rustc runs natively; the x86_64 toolchain compiles C and links) | PASS |
| `scripts/package-inoink.sh` produces `inoink-<v>-kobo.tar.gz`, `-source.tar.gz`, `-third-party-sources.tar`, `SHA256SUMS` | PASS |
| The binary is a 32-bit ARM hard-float ELF needing GLIBC <= 2.18 and the bundled libraries | PASS (readelf/objdump) |
| `scripts/check-reproducibility.sh`: two clean builds on this computer give byte-identical artifacts | PASS on one machine; not checked on another host or architecture |
| Package contents: licenses and notices present, no credentials, no unsafe paths, root-owned entries | PASS (`tests/package.sh <package>`) |
| Binary actually runs on the Kobo and loads its libraries | NOT RUN (device) |

## Emulator

`scripts/build-inoink.sh --run-emulator --data target/inoink-fixtures` (Libra Colour geometry,
1264x1680) replays a scripted tour and writes screenshots to `target/inoink-emulator-shots`:
list opens on Starred; article opens at its first page; page turns; the bar (Back, Star, Done,
More); the More menu; back to the list; a photo essay with colour images, captions and a retryable
placeholder for an image that failed. Positions are saved between runs. Each run works on a fresh
copy of the fixtures. Result: PASS. Not covered by the tour (tested headlessly instead): Done/Undo,
search, prompts, the enlargement view. The SDL dummy driver renders the framebuffer in software:
this says nothing about e-ink refresh, colour accuracy or touch hardware.

## Device (Kobo Libra Colour, firmware 4.46.23852)

None of these has been run. They need the physical device and your Inoreader credentials.
Record date, binary SHA-256 (`dist/SHA256SUMS`), model and firmware beside each result.

| Case | Result |
|---|---|
| Launch from NickelMenu; quit restores Nickel (normal exit and after a forced error) | NOT RUN |
| Physical page buttons and touch navigation | NOT RUN |
| Colour images render in articles and in the enlargement view | NOT RUN |
| Frontlight and warmth | NOT RUN |
| Rotation (portrait and landscape) | NOT RUN |
| Ten suspend/wake cycles; no sync starts while suspended | NOT RUN |
| USB share and unshare; the database opens afterwards | NOT RUN |
| Read downloaded articles and images with Wi-Fi off | NOT RUN |
| Done offline, restart, reconnect: read and unstarred on inoreader.com | NOT RUN |
| Mark read keeps the star | NOT RUN |
| Rapid toggles of read/star/tags end in the final state | NOT RUN |
| Star an old article elsewhere: it arrives and is cached | NOT RUN |
| Remote unstar and folder/rename changes arrive | NOT RUN |
| Interrupted image download and write recovery (pull Wi-Fi mid-sync) | NOT RUN |
| Expired token and quota exhaustion (watch the status line) | NOT RUN |
| Storage pressure behaviour | NOT RUN |
| Brightness and warmth match Nickel's at launch | NOT RUN |
| Left-edge slide changes brightness without turning pages | NOT RUN |
| Text panel: size, line spacing, margins, font and justification apply to all articles and survive a restart | NOT RUN |
| Network is up shown once per connection | NOT RUN |
| Page-turn latency with and without a running sync (same article) | NOT RUN |
| Update preserves data (install a second build over the first) | NOT RUN |
| Bulk "Mark list read" does not touch articles that arrive meanwhile | NOT RUN |
| Live API contract (`inoink-cli acceptance --live`, read-only, run from the computer) | PASS 2026-11-11: all 7 read endpoints; real limits 2000 per zone (zone 1 usage 154 before the run) |

## Known gaps

- The first live account has 1938 subscriptions and 38 folders/tags. The drawer's Feeds list pages
  ten entries at a time, which is impractical at that size; folders are the usable route.
- The first live article fetched had 23 characters of content: some feeds deliver almost nothing,
  which is what Load original page is for.

- No live Inoreader account was available: request shapes follow the published documentation;
  the first live run may reveal differences (for example whether `categories` omit state tags
  under `includeAllDirectStreamIds=false`, which InoInk deliberately never sends on `contents`).
- Feed ordering (`preference/stream/set`) is implemented in the sync layer and tested, but the
  interface has no reordering control yet. There is no "sign out" control; delete
  `.adds/inoink/data/credentials.json` to sign out.
- The Atom representation of streams is not implemented (JSON only, as Inoreader advises).
- `inoink-cli acceptance --live` makes read-only checks only.
- Hyphenation patterns and dictionaries are not shipped (license audit pending), so hyphenation is off.
- SVG images are not rendered (shown as retryable placeholders).
- Sleep-cover and screen-cover images from Plato's library are not available; the logo text is used.
