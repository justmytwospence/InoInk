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
| Local store: entity decoding of titles (ingest and migration), durable intent, Done vs Mark read, undo before/after acknowledgement, toggles, generation racing acknowledgement, independent tags, frozen bulk scope, stale snapshots, reconciliation, migrations, disk full, accounts | `crates/inoink/tests/store_intent.rs` (33) | PASS |
| API: every supported request shape, IDs (signed/hex), pagination parameters, encoding, Active Search incl. HTTP 202 and destructive GET, token refresh once, revocation, malformed/oversized responses, quota zones, reserve, 429 and persistence | `crates/inoink/tests/api_contract.rs` (24) | PASS |
| Sync faults: first sync incl. old starred, old star elsewhere, remote unstar, Done offline then restart, lost response, partial failure, crash in flight, toggles, tags, pending-wins-then-remote-wins, partial/failed/looping/capped listings, second-listing confirmation, silent unread boundary, token rotation/revocation, quota pause and resume, zone independence, interval, organization changes incl. lost responses, storage pressure, history paging, skipping unchanged listings, organization refresh on change | `crates/inoink/tests/sync_faults.rs` (32) | PASS |
| Content and cache: lazy/responsive/relative images, order and captions, sanitizing, malformed markup, tables/code, normalization and limits, decode bombs, bad MIME, SSRF and redirects, retries, concurrency, cancellation, leases, recovery of interrupted writes, symlinks, rooted paths, eviction and protection, plan priorities, low space, extraction, formulas (KaTeX, MathJax 2/3, MathML, Wikipedia, raw TeX, fallbacks), SVG rasterizing | `crates/inoink/tests/content_cache.rs` (24) | PASS |
| Lifecycle: UI reads/writes stay fast during slow sync, suspend cancels and quiesces, hung network does not hold suspend, no timers when idle, flapping network coalesced, debounce, USB closes the database fully and reopens, pending work survives shutdown, browse/open online | `crates/inoink/tests/lifecycle.rs` (9) | PASS |
| Highlights: Inoreader offsets (UTF-16) and quote anchoring (hyphenation, repeats, hints, missing text), ids and timestamps, Markdown and Readwise-shaped export | `crates/inoink/tests/highlights.rs` (13) | PASS |
| Documents and views: rooted resources (incl. symlinks), image hit data, offsets under font/rotation change, defaults, Done/Undo/Mark read, opening does not mark read, failed save, frozen list scope, tags, article swipes, page buttons and titles row, word-boundary truncation | `crates/core/tests/inoink_document.rs` (3), `inoink_views.rs` (30), `font_crop.rs` (2), in the container | PASS |
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
the Newsfeed with its end-of-list card; saving by tap; Categories and a folder; Read later; an
article with the bar and the page indicator; the next article (swipe up); a QR code; the light
panel; the main menu; the Newsfeed after saving. Screenshot `06-article` shows an Inoreader annotation
underlined in paragraph 2 (placed by context, the sentence repeats in every paragraph) and a
highlight made on the Kobo shaded in paragraph 1; `06b-details` shows the Details page (the Inoreader section is on its later pages, which the
tour does not turn to); `06c-selection-menu` and `06d-highlight` hold a word and highlight it,
and the highlight is then in the database (`inoink-cli highlights --data target/inoink-emulator-data`). Each run works on a fresh copy of the fixtures.
Result: NOT RUN since the end card was removed (12 screenshots expected). Not covered by the tour (tested headlessly
instead): Done/Undo, search, prompts, the enlargement view, link reading views, Karakeep. The SDL dummy driver renders the framebuffer in software:
this says nothing about e-ink refresh, colour accuracy or touch hardware.

## Device (Kobo Libra Colour, firmware 4.46.23852)

They need the physical device and your Inoreader credentials. Record date, build (Main menu >
Status > Version, or the binary SHA-256 from `dist/SHA256SUMS`), model and firmware beside each
result. Dev builds installed with `scripts/kobo-dev.sh` count when the build named is the one
that was merged.

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
| Expired token and quota exhaustion (watch Main menu > Status) | NOT RUN |
| Sync icon shows a dot and turns during a sync, and stops upright without the dot | NOT RUN |
| Settings > Inoreader API budget matches Inoreader's headers; a sync with nothing new uses 1-2 requests | NOT RUN |
| A change made on inoreader.com (read, star, new feed) arrives at the next sync despite skipped listings | NOT RUN |
| List footer "N / M" matches the pages you can turn to; the bar fills as you page | NOT RUN |
| Page bar: segments in portrait vs landscape follow the width (e.g. 15 pages: line in portrait, segments in landscape) | NOT RUN |
| Article bar: short articles show one segment per page, long ones the line; it follows font size changes | NOT RUN |
| Categories sort (Inoreader order, name, most unread) applies and is kept across restarts | NOT RUN |
| Storage pressure behaviour | NOT RUN |
| Brightness and warmth match Nickel's at launch | NOT RUN |
| Left-edge slide changes brightness without turning pages | NOT RUN |
| Text panel: size, line spacing, margins, font and justification apply to all articles and survive a restart | NOT RUN |
| Wi-Fi icon in the top bar follows the connection (no popup) | NOT RUN |
| Group and sort per view, kept across restarts; paging is stable while syncing | NOT RUN |
| Hacker News and Lobsters link posts: full article, comments switch, offline | NOT RUN |
| Page-turn latency with and without a running sync (same article) | NOT RUN |
| Update preserves data (install a second build over the first) | NOT RUN |
| Bulk "Mark list read" does not touch articles that arrive meanwhile | NOT RUN |
| Light inherited from Nickel at launch (check the `launch:` line in `inoink.log`: sysfs value, Kobo setting, result) | NOT RUN |
| Wi-Fi state inherited at launch, both on and off | NOT RUN |
| Wi-Fi icon shows off, connecting and online; tapping it toggles Wi-Fi | NOT RUN |
| Light panel: on/off, brightness and warmth steppers and sliders; the edge slide still works | NOT RUN |
| Tabs and counts (Newsfeed, Categories, Read later) agree with the lists | NOT RUN |
| Tap-to-save in the Newsfeed; hold opens; the setting swaps them | NOT RUN |
| End of unread list: "Mark N read" marks only what was seen and not starred | NOT RUN |
| Article tap zones (left, middle, right thirds) with no Plato corner gestures firing | NOT RUN |
| Paging past the last page, swipe up and holding forward open the next article; swipe down and holding back the previous one, where it was left; the release never turns a page in the new article | NOT RUN |
| Done (bar) in Read later archives it on inoreader.com; changing articles marks nothing read | NOT RUN |
| Short or slanted vertical swipes do nothing; a double swipe opens one article; paging back from page 1 opens the previous article's last page | NOT RUN |
| Switching articles shows no blank page before the new article | NOT RUN |
| An article opened part-way shows the bar until the first page turn | NOT RUN |
| Inverted page-turn buttons also swap the swipe directions | NOT RUN |
| Long titles end at a whole word with an ellipsis (lists, bar, titles row); titles with dashes and ampersands show the characters, not `&mdash;`/`&amp;` | NOT RUN |
| Titles row with the bar (tap opens); Always: titles beside the progress line are legible and clear of the text at the default margin | NOT RUN |
| Tapping a link opens a reading view; holding shows the link menu | NOT RUN |
| QR code scans with a phone | NOT RUN |
| Follow in Inoreader adds a feed from a held link | NOT RUN |
| Karakeep: saved online, and queued offline then sent after reconnecting | NOT RUN |
| Highlight, add a note, edit and remove in an article; they survive leaving and restarting | NOT RUN |
| An Inoreader annotation is underlined at the right place, and holding it shows its note | NOT RUN |
| Highlights are kept after Load original page | NOT RUN |
| Inline and display formulas: right size and baseline, follow font size changes | NOT RUN |
| A math-heavy article caches in reasonable time (`math:` line in inoink.log) | NOT RUN |
| SVG images show in articles | NOT RUN |
| `inoink-cli highlights --data` on the USB-mounted data directory lists the highlights | NOT RUN |
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
- Inline `<svg>` markup inside articles is not shown (SVG image files are).
- Formulas that use macros a site defines for itself, or LaTeX packages MiTeX does not know, show as LaTeX source.
- Articles cached before formula support keep their old text until re-downloaded.
- Sleep-cover and screen-cover images from Plato's library are not available; the logo text is used.
