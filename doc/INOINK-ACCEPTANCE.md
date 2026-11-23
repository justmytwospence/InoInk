# Acceptance evidence

Three kinds of evidence are kept apart. **Automated** means unit, integration and fault tests run
on the development computer with a simulated Inoreader. **Emulator** means the desktop build under
SDL's dummy video driver or the native interactive preview. **Device** means a physical Kobo Libra Colour. Emulator results are never
evidence about hardware.

Status words: PASS (done, evidence named), NOT RUN (not performed), N/A.

## Automated

Run with `scripts/test-inoink.sh` (format check, library and CLI tests, lints, native view and
document tests inside the build container, and the install/launcher tests).

| Area | Where | Result |
|---|---|---|
| Local store: entity decoding of titles (ingest and migration), durable intent, Done vs Mark read, undo before/after acknowledgement, toggles, generation racing acknowledgement, independent tags, frozen bulk scope, stale snapshots, reconciliation, migrations, disk full, accounts, seen and new, bulk undo, highlight styles and colours (incl. older highlights staying gray shading) | `crates/inoink/tests/store_intent.rs` (43) | PASS |
| API: every supported request shape, IDs (signed/hex), pagination parameters, encoding, Active Search incl. HTTP 202 and destructive GET, token refresh once, revocation, malformed/oversized responses, quota zones, reserve, 429 and persistence | `crates/inoink/tests/api_contract.rs` (24) | PASS |
| Sync faults: first sync incl. old starred, old star elsewhere, remote unstar, Done offline then restart, lost response, partial failure, crash in flight, toggles, tags, pending-wins-then-remote-wins, partial/failed/looping/capped listings, second-listing confirmation, silent unread boundary, token rotation/revocation, quota pause and resume, zone independence, interval, organization changes incl. lost responses, storage pressure, history paging, skipping unchanged listings, organization refresh on change | `crates/inoink/tests/sync_faults.rs` (32) | PASS |
| Content and cache: teaser pages replaced by the original text (per-feed learning), lazy/responsive/relative images, order and captions, sanitizing, malformed markup, tables/code, normalization and limits, decode bombs, bad MIME, SSRF and redirects, retries, concurrency, cancellation, leases, recovery of interrupted writes, symlinks, rooted paths, eviction and protection, plan priorities, low space, extraction, formulas (KaTeX, MathJax 2/3, MathML, Wikipedia, raw TeX, fallbacks), SVG rasterizing | `crates/inoink/tests/content_cache.rs` (33) | PASS |
| Lifecycle: UI reads/writes stay fast during slow sync, suspend cancels and quiesces, hung network does not hold suspend, no timers when idle, flapping network coalesced, debounce, USB closes the database fully and reopens, pending work survives shutdown, browse/open online | `crates/inoink/tests/lifecycle.rs` (10) | PASS |
| Wi-Fi policy and failures (worker side): Wait holds syncs until the network is up and never asks for the Wi-Fi; Turn on asks, syncs, sends changes made meanwhile, hands it back, gives up after the deadline, throttles local changes, respects the user switching it off, follows a setting change; 3 failures in a row stop automatic syncs until a tap or a wake, a success resets the count, connections that never come count | `crates/inoink/tests/lifecycle.rs` (12), `inoink_views.rs` (`wifi_policy_defaults_to_wait_and_is_saved`) | PASS |
| Discussions: Hacker News and Lobsters detection, thread parsing (tree order, depths, flat lists), well-formed comments page, nesting capped at two indents with parent names, OP, reply counts, Skip links, comment ages | `crates/inoink/tests/discussion.rs` (6) | PASS |
| Link reading views: text and title extraction, https and HTML only, short-text reporting, og:title, prepared article shape | `crates/inoink/tests/web_page.rs` (5) | PASS |
| Karakeep: queue lifecycle and dedupe, request shape, status mapping, encrypted config round trip, flush stops on retry | `crates/inoink/tests/share.rs` (5) | PASS |
| List views: counts, query order and cap, unread per folder, folder ordering, per-collection defaults, word counts, time left, mark read up to / from here | `crates/inoink/tests/views.rs` (8) | PASS |
| Preview snapshot: exact read-only allowlist, forbidden writes never reach the network, read scope only, token rotation, bounded and atomic publication, no partial or unreadable snapshots, no private URLs logged | `crates/inoink/tests/snapshot.rs` (23), `crates/inoink-cli/tests/snapshot_pty.rs` (2) | PASS |
| Highlights: Inoreader offsets (UTF-16) and quote anchoring (hyphenation, repeats, hints, missing text), ids and timestamps, Markdown, HTML (escaping, styles, safe links) and Readwise-shaped export, export file names, style and colour names | `crates/inoink/tests/highlights.rs` (16) | PASS |
| Documents and views: rooted resources (incl. symlinks), image hit data, offsets under font/rotation change, defaults, Done/Undo/Mark read, opening does not mark read, failed save, frozen list scope, tags, article swipes, page buttons and footer chevrons, word-boundary truncation, new marks per visit, selection mode, end-of-article popup (state, Up next, swipe on, grace, Back to list), one toast at a time, back by title, running headers (title, thread), left borders across pages | `crates/core/tests/inoink_document.rs` (5), `inoink_views.rs` (68), `font_crop.rs` (2), in the container | PASS |
| Reader tools: image viewer zoom (pinch, buttons, limit), drag to pan, double-tap, per-image memory; Return after a tapped link, a footnote jump, a Contents jump and an Article / comments switch (not after the comments' Skip), hold to dismiss, paging back clears it; heading contents; highlight shading, tint, underline and dotted underline pixels; style and colour saved and reused; on-device export to Markdown and HTML | `crates/core/tests/inoink_reader_tools.rs` (12), in the container | PASS |
| Comments button: count labels incl. 0 and 999+, toggle in both directions, article icon without count on the comments, layout in portrait/landscape/narrow, star index kept | `crates/core/tests/inoink_views.rs`, `article_bar.rs` unit test, in the container | PASS 2026-11-15 |
| Sleep screens: setting default and TOML round trip, Settings > Sleep screen entries and choice, card counts, last sync (this session and after a restart), battery and pending lines, status card and dark power-off drawn, current page left alone with its label, PNG over the page (opaque, transparent, hidden and broken files skipped), Logo keeps Plato's screens, picture scaling with alpha | `crates/core/tests/sleep_screen.rs` (7), `crates/core/src/view/inoink/sleep.rs` (6), in the container | PASS |
| Reading fonts: the bundled families match `fonts/reading`, each loads with Regular, Italic, Bold and Bold Italic, families resolve to the right directory, the Aa font menu lists the default, bundled and user fonts (a user copy of a bundled family once), choosing a bundled family changes the layout and the default restores it | `crates/core/tests/reading_fonts.rs` (5), in the container | PASS 2026-11-23 |
| Device detection: Kobo and Tolino codenames map to their models, unknown codenames fall back to a current 300 dpi model, no-frontlight models, experimental (old kernel) models | `crates/core/src/device.rs` (`codenames_and_aliases`), in the container | PASS |
| Install, update, rollback, removal, launcher (Nickel restored once on exit, failure, SIGTERM; double launch; reboot intent; MediaTek reboot unless the Wi-Fi stayed off, stale markers ignored), package safety, the `device:` line in the log | `tests/package.sh` | PASS |
| Start-up self-check, install side: release manifest, checksums verified at install (mismatched unpack not installed), missing or resized file or a loader failure (`LOADFAIL`) falls back to the previous release with a notice, nothing usable leaves Nickel running, `--check`/`--why` for the NickelMenu entry (message within NickelMenu's 500 characters), installer refreshes an existing entry | `tests/package.sh` (also under BusyBox 1.31 in Docker) | PASS 2026-11-23 |
| Start-up self-check, device side: tested/untested firmware and model, Tolino and unknown models, firmware 5+, one notice per model and firmware, README Status names exactly the tested list, model tiers | `crates/inoink/src/device_check.rs` (4), `crates/core/src/device.rs` (`codenames_and_aliases`) | PASS 2026-11-23 |
| Inherited Plato unit tests | `crates/core` | PASS (38) |
| Browser installer: writes the same files as `install-inoink.sh` (compared file for file) and the Kobo's launcher installs them; damaged copy discarded; no launcher entry without NickelMenu; removal at three levels matches `remove-inoink.sh`; `cksum` and SHA-256 match the system tools; tar formats (ustar, pax, GNU long names); unsafe, incomplete and foreign packages refused; `.kobo/version` parsing, firmware limits, device list agrees with the README; release lookup and verification; blocked download reported | `tests/web-installer.test.mjs` (11, Node) | PASS |
| Screen review tool: rounds and carry-over of verdicts, replies, the queue, Before mappings, the notes output; the server takes feedback for the latest round only, needs JSON from its own origin and refuses other host names | `scripts/review/review.test.mjs` (9), Node.js | PASS 2026-11-23 |
| Device benchmark tool: touch coordinates against InoInk's device table (every supported model and rotation), input event layout, screen signatures, settings and the shipped `bench.cfg`, the startup log, the report, whole runs against a simulated InoInk (including runs that must stop) | `crates/inoink-bench` (21), macOS and container Linux | PASS 2026-11-23 |
| `scripts/kobo-bench.sh` copies only short names, keeps edited settings, collects once, refuses to remove uncollected results, never touches InoInk | `tests/kobo-bench.sh` | PASS 2026-11-23 |

## Build and package (computer only)

| Check | Result |
|---|---|
| `scripts/build-inoink.sh --target kobo` cross-builds the ARM binary with the pinned Linaro GCC 4.9.4 toolchain (rustc runs natively; the x86_64 toolchain compiles C and links) | PASS |
| `scripts/package-inoink.sh` produces `inoink-<v>-kobo.tar.gz`, `-source.tar.gz`, `-third-party-sources.tar`, `SHA256SUMS` | PASS |
| The binary is a 32-bit ARM hard-float ELF needing GLIBC <= 2.18 and the bundled libraries | PASS (readelf/objdump) |
| `scripts/check-reproducibility.sh`: two clean builds on this computer give byte-identical artifacts | PASS on one machine; not checked on another host or architecture |
| Package contents: licenses and notices present, no credentials, no unsafe paths, root-owned entries | PASS (`tests/package.sh <package>`) |
| Binary actually runs on the Kobo and loads its libraries | NOT RUN (device) |

## Browser installer (computer only)

`node tests/web-installer-chrome.mjs inoink-0.11.0-kobo.tar.gz` drove the project page in headless
Chrome 158 on macOS 27.2 on 2026-11-23, with a fake Libra Colour (`.kobo/version`, NickelMenu
folder) in the browser's private file system: Chrome's real File System Access, decompression and
hashing, and the live GitHub API. The native folder picker cannot be automated, so a real USB
drive was not used.

| Check | Result |
|---|---|
| The page recognises the model and firmware, refuses a folder that is not a Kobo, and names the latest release | PASS |
| Downloading the release from GitHub is blocked by CORS (`release-assets.githubusercontent.com` sends no `Access-Control-Allow-Origin`), and the page asks for the downloaded file instead | PASS (Chrome's console names the CORS block) |
| Installing from the chosen 0.11.0 package: `INSTALL.TGZ` byte-identical, `INSTALL.SUM` equal to `cksum`, launcher and NickelMenu entry written, no swap files left; about 2 s | PASS |
| A modified package is refused | PASS |
| Remove, keeping data, then everything | PASS |
| Inoreader's token endpoint and API send no CORS headers (`curl` with an `Origin` header, preflight and POST), so sign-in stays in `inoink-cli` | PASS (checked 2026-11-23) |
| Install and remove on a real KOBOeReader drive from Chrome or Edge (folder picker, permission prompt, FAT writes) | NOT RUN |

## Emulator

`scripts/build-inoink.sh --run-emulator --data target/inoink-fixtures` (Libra Colour geometry,
1264x1680) replays a scripted tour and writes screenshots to `target/inoink-emulator-shots`:
the Newsfeed with its end-of-list card; saving by tap; a headline's menu; the Newsfeed's folder list and a folder; Read later and its Sort menu (`05b-sort-menu`); an
article with the bar, the next article's title and the page indicator; the next article (swipe up); its end-of-article popup (`09b-article-end`); a QR code; the light
panel; the main menu; the Newsfeed after saving; a Hacker News link post as the article (`14-linked-article-tabs`) and its comments with thread lines (`15-linked-comments-tabs`). Screenshot `06-article` shows an Inoreader annotation
underlined in paragraph 2 (placed by context, the sentence repeats in every paragraph) and a
highlight made on the Kobo shaded in paragraph 1; `06b-details` shows the Details page (the Inoreader section is on its later pages, which the
tour does not turn to); `06c-selection-menu` and `06d-highlight` hold a word and highlight it,
and the highlight is then in the database (`inoink-cli highlights --data target/inoink-emulator-data`). Each run works on a fresh copy of the fixtures.
Result: PASS (19 screenshots inspected on 2026-11-16). Not covered by the tour (tested headlessly
instead): Done/Undo, search, prompts, the enlargement view, link reading views, Karakeep. The SDL dummy driver renders the framebuffer in software:
this says nothing about e-ink refresh, colour accuracy or touch hardware.

Start-up notice (run by hand on 2026-11-23, Libra Colour geometry, with a local tour that taps
through it): with `FIRMWARE_VERSION=4.99.1` the untested-firmware page opens over the list, the
middle tap shows the reader bar and its back arrow reaches the Newsfeed with its articles; with
6.0.279225 the page adds the firmware 5+ paragraph; a second start on the same firmware shows no
page; `INOINK_NOTICE` shows as a toast over the list. Result: PASS (emulator only).

### Native macOS interactive preview

`scripts/preview-macos.sh` builds the emulator natively on Apple Silicon macOS, with cached
MuPDF 1.27.0 and Homebrew libraries. It opens a half-size window backed by the full 1264x1680
Libra Colour framebuffer, with mouse input mapped into device coordinates. It uses offline
fixtures and keeps mutable data, settings and screenshots under `target/macos-preview/state`;
`--reset` resets only that state. Native dependencies and incremental Cargo artifacts are
cached separately under `target/macos-preview`.

| Check | Result |
|---|---|
| Native dependency preparation and emulator/CLI build on macOS arm64 | PASS 2026-11-15 (macOS 27.2, build 26B123; Rust 1.99.1; Apple clang 21; MuPDF 1.27.0; SDL2-compat 2.32.74); repeated build reuses native cache |
| Native dummy-driver tours at scales 0.5 and 1.0; full-resolution home/article/menu screenshots | PASS 2026-11-15 (macOS 27.2 arm64): 16 PNGs per tour at 1264x1680; home/article/menu inspected; article PNG identical between scales |
| Portable launcher tests and emulator framebuffer/input regression tests | PASS 2026-11-15: launcher tests on macOS and container Linux; 7 emulator tests in the container |
| Reduced preview windows area-average pixels so one-pixel lines stay visible (even/odd phases, noninteger ratios, real SDL canvas readback); 1.0 exact, 2.0 nearest | PASS 2026-11-15: 10 emulator tests in the container |
| Native dummy-driver tours at scales 0.5 and 0.4 with link-post `14-linked-article-tabs` and `15-linked-comments-tabs` | PASS 2026-11-15 (macOS arm64): 18 PNGs per tour at 1264x1680; comments icon in both states inspected, no clipped or clashing controls |
| Comments icon looks right in the visible reduced window | NOT RUN (human visual check) |
| Visible window fits; click/hold/drag and B/F navigate; menu Undo works | NOT RUN (human interaction) |
| Rotation keeps input aligned; S saves a full-resolution screenshot; Escape/window close quits | NOT RUN (human interaction) |
| Relaunch retains sample changes/settings; `--reset` restores fresh state | NOT RUN (human interaction) |
| `--capture`: the tour without a window on fresh sample data (or a copy of the snapshot) with default settings; the preview's state untouched; only that device's earlier capture replaced; failures keep it | PASS 2026-11-23: `tests/preview-macos.sh` (macOS and container Linux); a real capture on macOS arm64 saved 19 PNGs at 1264x1680 |
| Screen review page in a browser: pins, notes, comparison, rounds | See [screen review](#screen-review) |

The native article screenshot was also compared visually with the existing Linux-container
Libra Colour tour: layout and content match, with no observed clipping or corruption. The native
and container PNGs are not byte-identical; native library/rasterization differences are not a
hardware fidelity guarantee.

These checks do not establish e-ink refresh timing, ghosting, colour accuracy, physical touch,
page-button behavior, suspend or other device integration.

### Real-article snapshot sandbox

The opt-in `--inoreader` preview uses a separately authorized read-only snapshot and runs the UI
offline. Automated endpoint-denial and sandbox tests use simulated accounts; they do not prove
behavior against the live service. OAuth token refresh and publisher image/article requests may
occur while downloading, but article and account state must not be changed.

| Check | Result |
|---|---|
| Read-only endpoint/scope enforcement, bounded download, isolated credentials, offline UI and launcher cancellation | PASS (automated mocks, core view tests and terminal tests; `scripts/test-inoink.sh`, macOS and container Linux) |
| Live read-only authorization and snapshot retrieval on macOS | NOT RUN |
| Mark read, star, Archive and Undo in preview; compare actual Inoreader state | NOT RUN (human interaction) |
| Relaunch preserves local experiments; refresh replaces them without affecting account | NOT RUN (human interaction) |

### Screen review

[INOINK-SCREEN-REVIEW.md](INOINK-SCREEN-REVIEW.md) shows captured screens one at a time with
notes pinned to them. Automated checks are listed above.

| Check | Result |
|---|---|
| A round from a real capture (19 screens) in headless Chrome, driven by a script: the queue filter on first open, a pin and its note saved to `feedback.json`, Right marks seen, side by side with Before, a newer round turns the page read only with a banner, Open loads it, the changed screen shows the earlier notes and its differences | PASS 2026-11-23 (macOS arm64, Chrome, Node 23) |
| The same by hand: layout at several window sizes, Hold to compare, zoom, finishing a round | NOT RUN (human interaction) |

### Screen geometries

`scripts/emulator-matrix.sh` runs the same tour as each device (`--device CODENAME:MODEL`) and
writes screenshots to `target/inoink-emulator-shots/<codename>`. Inspected on 2026-11-13 (01, 07
and 11 for each; other shots were produced without errors). Same caveat: nothing here is about
hardware.

| Device pretended | Screen | Result |
|---|---|---|
| monza (Libra Colour) | 1264x1680 @ 300 | PASS |
| spaBW (Clara BW) | 1072x1448 @ 300 | PASS |
| condor (Elipsa 2E) | 1404x1872 @ 227 | PASS with a known issue: rows are sized in dp, so at 227 dpi the list fills only the upper part of the screen |
| frost (Forma) | 1440x1920 @ 300 | PASS |
| luna (Nia) | 758x1024 @ 212 | PASS |
| pika (Touch 2.0) | 600x800 @ 167 | PASS: no light icon in the top bar. The tour's `11-light-panel` still shows the panel because the emulator opens it itself |
| trilogy (Touch C) | 600x800 @ 167, single touch | PASS: no light icon |

## Device (Kobo Libra Colour, firmware 4.46.23852)

They need the physical device and your Inoreader credentials. Record date, build (Main menu >
Status > Version, or the binary SHA-256 from `dist/SHA256SUMS`), model and firmware beside each
result. Dev builds installed with `scripts/kobo-dev.sh` count when the build named is the one
that was merged.

| Case | Result |
|---|---|
| Launch from NickelMenu; quit restores Nickel (normal exit and after a forced error) | NOT RUN |
| Start-up check from the NickelMenu entry (`--check`, then start): no visible delay before "Opening InoInk" | NOT RUN |
| A file removed from the active release: InoInk starts the previous release and shows "InoInk <v> is damaged" | NOT RUN |
| No usable release: the Kobo interface stays, NickelMenu shows "InoInk can't start" with what is wrong; reinstalling fixes it | NOT RUN |
| Untested firmware (`export INOINK_CHECK_FIRMWARE=4.99.0` in `data/config.sh`): the notice page shows once, leaving it reaches the list, not shown on the next start; the `device check:` log line | NOT RUN |
| Launch shows "Opening InoInk" at once; the `startup:` lines in `inoink.log` show the list arriving no later than before | NOT RUN |
| Quit shows "Closing InoInk" at once ("Restarting the Kobo" when a MediaTek Kobo reboots); an open article keeps its position | NOT RUN |
| Physical page buttons and touch navigation | NOT RUN |
| Colour images render in articles and in the enlargement view | NOT RUN |
| Image viewer: pinch and spread zoom, Zoom out / Fit / Zoom in, drag to pan, double-tap fit and 2x; reopening the image keeps its zoom | NOT RUN |
| Frontlight and warmth | NOT RUN |
| Rotation (portrait and landscape) | NOT RUN |
| Ten suspend/wake cycles; no sync starts while suspended | NOT RUN |
| USB share and unshare; the database opens afterwards | NOT RUN |
| Read downloaded articles and images with Wi-Fi off | NOT RUN |
| Archive offline, restart, reconnect: in the Archive and out of Read later on inoreader.com, reading progress kept, read state unchanged | NOT RUN |
| Mark read keeps the star | NOT RUN |
| Rapid toggles of read/star/tags end in the final state | NOT RUN |
| Star an old article elsewhere: it arrives and is cached | NOT RUN |
| Remote unstar and folder/rename changes arrive | NOT RUN |
| Interrupted image download and write recovery (pull Wi-Fi mid-sync) | NOT RUN |
| Expired token and quota exhaustion (watch Main menu > Status) | NOT RUN |
| Sync icon shows its syncing form (dot) during a sync, without ghosting, and returns to the plain icon | NOT RUN |
| Top bar title is bold, centred and not cut short | NOT RUN |
| List pages: the page indicator sits in its own band between the headlines and the footer, menus open above it; group headers read as grey section bands | NOT RUN |
| Settings > Inoreader API budget matches Inoreader's headers; a sync with nothing new uses 1-2 requests | NOT RUN |
| A change made on inoreader.com (read, star, new feed) arrives at the next sync despite skipped listings | NOT RUN |
| List footer "N / M" matches the pages you can turn to; the bar fills as you page | NOT RUN |
| List footer: Show, Group and Sort labels each open their own menu with no "More" entry; the labels fit in portrait; the list still shows eight headlines on the Libra Colour | NOT RUN |
| Page bar: segments in portrait vs landscape follow the width (e.g. 15 pages: line in portrait, segments in landscape) | NOT RUN |
| Article bar: short articles show one segment per page, long ones the line; it follows font size changes | NOT RUN |
| Folder sort (Inoreader order, name, most unread) applies and is kept across restarts | NOT RUN |
| Storage pressure behaviour | NOT RUN |
| Brightness and warmth match Nickel's at launch | NOT RUN |
| Left-edge slide changes brightness without turning pages | NOT RUN |
| Text panel: size, line spacing, margins, font and justification apply to all articles and survive a restart | NOT RUN |
| Bundled reading fonts: Libron, NV Bitter, NV Jost and NV Literata appear in Aa > Font, set body text with italic and bold, and survive a restart; side-by-side with Libertinus Serif for the default | NOT RUN |
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
| Tabs (Newsfeed, Read later): the active tab names the view with its count; tapping it opens and closes its list; the other tab opens Newsfeed or Read later | NOT RUN |
| Archive (Read later > Archive) lists the same articles as Saved > Archive on inoreader.com, with reading progress | NOT RUN |
| Restore to Read later (menu, ribbon) moves an archived article back, progress kept; Remove from Archive asks first | NOT RUN |
| Read later's list shows Read later, Archive, Web pages, Annotations and your tags; the Newsfeed's shows Newsfeed, folders and monitoring feeds | NOT RUN |
| Tap opens a headline; tapping the ribbon at the right saves or unsaves it; hold opens the menu; the outlined ribbon reads clearly without ghosting | PASS 2026-11-13 (Libra Colour 4.46.23852, 0.6.0-dev.4.36eee1d) |
| End of unread list: "Mark N read" marks only what was seen and not saved, at once, with Undo | PASS 2026-11-13 (Libra Colour 4.46.23852, 0.6.0-dev.4.36eee1d) |
| Mark-read button in the top bar asks, then marks the whole list read except saved articles; Undo restores | PASS 2026-11-13 (Libra Colour 4.46.23852, 0.6.0-dev.4.36eee1d) |
| Mark read up to here / from here down follow the list's sort, include the held headline and skip saved articles | PASS 2026-11-13 (Libra Colour 4.46.23852, 0.6.0-dev.4.36eee1d) |
| After a bulk mark-read a sync starts at once and new unread articles arrive | PASS 2026-11-13 (Libra Colour 4.46.23852, 0.6.0-dev.4.36eee1d) |
| Top-bar select button enters selection mode with nothing ticked; pressing it again leaves | PASS 2026-11-13 (Libra Colour 4.46.23852, 0.6.0-dev.4.36eee1d) |
| Archive and bulk Read/Unread/Save/Unsave update silently with rows unobscured; Main menu > Undo last change remains available after paging and returning from a reader, even after waiting beyond six seconds, and restores silently without a timer | NOT RUN |
| Comments: thread lines continue across pages, two indents at most with ↳ beyond, ⁂ before each thread, reply count and Skip › on the first comment, OP marked, ages, Thread k of n in the top margin | NOT RUN |
| Article bar: the title is left-aligned and tapping it goes back like the arrow | NOT RUN |
| MediaTek quick quit: Wi-Fi off at launch and never turned on, Quit says "Quit", the closing screen says Closing InoInk and the Kobo interface returns without a reboot (repeat several times, then use USB and the Wi-Fi in Nickel normally); after turning the Wi-Fi on, the menu says Quit (the Kobo restarts) and the Kobo reboots | NOT RUN |
| Toasts appear centred just below the top bar, one at a time (a new one replaces the old); no "Full text" or "Saved to Karakeep" confirmations | NOT RUN |
| New bar: unseen unread headlines show a 4 dp left bar that stays while in the list and is gone after an article, waking or switching lists; it reads clearly without looking heavy | NOT RUN |
| Saved headlines show only the ribbon | NOT RUN |
| Selection mode: check boxes toggle without ghosting; hold selects a range; Read/Unread/Save/Unsave apply and Undo reverts; the footer labels fit in portrait and landscape | NOT RUN |
| Selection survives paging and a sync refresh; switching tabs cancels it | NOT RUN |
| Article tap zones (left, middle, right thirds) with no Plato corner gestures firing | NOT RUN |
| Swipe up and holding forward open the next article; swipe down and holding back the previous one, where it was left; the release never turns a page in the new article | NOT RUN |
| End-of-article popup: paging past the last page opens it; Mark read is preselected (Leave as is for a read article), Archive moves it to the Archive, leaving read state alone (Undo in the main menu), Keep unread leaves it; Archive is remembered for saved articles (preselected next time until another choice is made); Up next names the next article (End of list after the last); swiping on or forward applies the choice and opens it, but not in the first moment after the popup opens; Back to list applies it and returns, Next article beside it applies it and opens the next; swiping back, the back button and a tap outside close it; layout reads well | NOT RUN |
| Archive (bar) in Read later archives it on inoreader.com, progress kept; changing articles marks nothing read | NOT RUN |
| Short or slanted vertical swipes do nothing; a double swipe opens one article; paging back from page 1 opens the previous article's last page | NOT RUN |
| Switching articles shows no blank page before the new article | NOT RUN |
| Read later rows show Finished or "N% · M min left" matching inoreader.com; videos and podcasts show their duration | NOT RUN |
| Read later Show menu filters Not started / In progress / Finished; Recently saved orders as on inoreader.com | NOT RUN |
| An article started on inoreader.com opens near the same place; one read to the end on the Kobo shows Finished | NOT RUN |
| An article opened part-way shows the bar until the first page turn | NOT RUN |
| Curved and slightly diagonal vertical swipes change article; a stalled download shows "Still downloading" and gives way after 10 s | NOT RUN |
| The article title shows in the top margin of every page but the first, clear of the text | NOT RUN |
| Curly quotes, dashes and ellipses in articles (not in code); a German or French article hyphenates in its language | NOT RUN |
| Inverted page-turn buttons also swap the swipe directions | NOT RUN |
| Long titles end at a whole word with an ellipsis (lists, bar, end popup); titles with dashes and ampersands show the characters, not `&mdash;`/`&amp;` | NOT RUN |
| Hacker News links to sites that refused InoInk before (401/403) now download | NOT RUN |
| Sleep screen Status card (default): the power button and the sleep cover show "Sleeping" with the unread and Read later counts as in the tabs, "Synced today at ..." and the battery level; no Wi-Fi or sync starts for it; waking restores the list or article without ghosting | NOT RUN |
| Sleep screen Current page: an article page and the list stay as they were with the "Sleeping" label at the bottom, sharp after the full refresh, in portrait and landscape | NOT RUN |
| Sleep screen Picture over the page: a transparent PNG in `.adds/inoink/screensaver` sits on the page and an opaque one covers it, in colour on the Libra Colour and in greys on a black-and-white Kobo; with the folder empty the page shows alone and Settings > Sleep screen names the folder | NOT RUN |
| Sleep screen Logo: Plato's "Sleeping" screen, as before | NOT RUN |
| Power off (hold the power button): the white-on-black "Powered off" card stays on screen after the Kobo is off | NOT RUN |
| Sleep screens with Invert colors on: the card and pictures are readable and pictures keep their colours | NOT RUN |
| Teaser feeds open as the full page; Full text Never keeps the teaser | NOT RUN |
| Footer chevrons with the bar (tap a side opens that article; none at the ends of the list); Always: titles beside the progress line are legible and clear of the text at the default margin | NOT RUN |
| Tapping a link opens a reading view; holding shows the link menu | NOT RUN |
| QR code scans with a phone | NOT RUN |
| Follow in Inoreader adds a feed from a held link | NOT RUN |
| Karakeep: saved online, and queued offline then sent after reconnecting | NOT RUN |
| Highlight, add a note, edit and remove in an article; they survive leaving and restarting | NOT RUN |
| An Inoreader annotation is underlined at the right place, and holding it shows its note | NOT RUN |
| Highlights are kept after Load original page | NOT RUN |
| Highlight Style (shading, underline, dotted) and Colour (colour screens only) apply at once, survive leaving and restarting, and become the look of the next highlight; colours read on the Libra Colour | NOT RUN |
| More > Export highlights > Markdown and HTML write files to `.adds/inoink/exports` that open on a computer | NOT RUN |
| Return: a footnote or in-article link, a heading from More > Contents, and the Article / comments switch each show Return in the top margin; tap returns, hold dismisses | NOT RUN |
| Load original page on a teaser swaps the text in place, from the top, without reopening the article | NOT RUN |
| Inline and display formulas: right size and baseline, follow font size changes | NOT RUN |
| A math-heavy article caches in reasonable time (`math:` line in inoink.log) | NOT RUN |
| SVG images show in articles | NOT RUN |
| `inoink-cli highlights --data` on the USB-mounted data directory lists the highlights | NOT RUN |
| Browser installer, Chrome on macOS: the page shows the model, firmware, installed version and NickelMenu; Install writes `.adds/inoink/INSTALL.TGZ`, `INSTALL.SUM`, `current/inoink.sh` and `.adds/nm/inoink`; after ejecting, InoInk starts from NickelMenu and `inoink.log` says "installed release"; no `FSCK0000.000` or `.crswap` files appear after the Kobo's disk check | NOT RUN |
| Browser installer update over an installed release keeps data and sign-in, and the previous release stays for rollback | NOT RUN |
| Browser installer removal: keep data, reset settings and delete everything each leave the Kobo as described, and the NickelMenu entry goes | NOT RUN |
| Wait for Wi-Fi (default), Wi-Fi off: no sync and no Wi-Fi at launch or wake; the status says Offline; tapping sync says "Turn on Wi-Fi to sync"; turning the Wi-Fi on syncs once (and sends changes made offline) | NOT RUN |
| Turn on Wi-Fi to sync, Wi-Fi off: launch, wake and a tap on sync turn the Wi-Fi on (icon connecting, then online; status "Turning on Wi-Fi"), sync, and turn it off again; `inoink.log` shows `wifi: on for a sync` / `wifi: off after the sync` | NOT RUN |
| Turn on Wi-Fi to sync, Wi-Fi switched on with the icon (or on at launch): it stays on after syncs and comes back after a wake | NOT RUN |
| Turn on Wi-Fi to sync: a change made offline turns the Wi-Fi on at most every 15 minutes; tapping the icon off while it connects leaves it off | NOT RUN |
| Turn on Wi-Fi to sync with no reachable network: the Wi-Fi goes off after about a minute; after three such attempts the status says "Sync stopped after 3 failures: tap sync to retry" and nothing turns the Wi-Fi on until a wake or a tap | NOT RUN |
| Turn on Wi-Fi to sync on a MediaTek Kobo: the menu says Quit (the Kobo restarts) after the launch sync, and quitting restarts the Kobo cleanly | NOT RUN |
| Sleep during a sync that turned the Wi-Fi on: the Wi-Fi is off during sleep and not back at wake until the wake sync asks for it | NOT RUN |
| Live API contract (`inoink-cli acceptance --live`, read-only, run from the computer) | PASS 2026-11-11: all 7 read endpoints; real limits 2000 per zone (zone 1 usage 154 before the run) |

## Device benchmarks

Timings from a real Kobo with `inoink-bench` ([INOINK-BENCHMARKS.md](INOINK-BENCHMARKS.md)), one
table per release, pasted from `scripts/kobo-bench.sh collect`. Times run from the input to the
framebuffer (the e-ink refresh is not included); compare releases on the same device and firmware
only. No emulator or simulated numbers belong here.

| Check on the Libra Colour | Result |
|---|---|
| `scripts/kobo-bench.sh install` copies the benchmark and its menu entry; the Kobo's disk check leaves them alone | NOT RUN |
| **InoInk benchmark** in NickelMenu starts InoInk and the run; the run waits for the list and the startup sync | NOT RUN |
| Injected page-button presses turn list and article pages | NOT RUN |
| Injected taps land where aimed: the first row, the sync button, the middle of an article, its title | NOT RUN |
| The run ends with an `end` line, InoInk stays usable, and `collect` prints the table | NOT RUN |

### Releases

No device run yet. Add each release as `#### <version> (<model>, firmware <firmware>)` with the
date, the number of runs and the table.

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

## Other devices

Filled in from Device report issues; a row says PASS only for what the report covers.

| Model | Firmware | Build | Result | Report |
|---|---|---|---|---|
| Clara Colour | | | NOT RUN | |
| Clara BW | | | NOT RUN | |
| Elipsa 2E | | | NOT RUN | |
| Clara 2E | | | NOT RUN | |
| Libra 2 | | | NOT RUN | |
| Sage | | | NOT RUN | |
| Elipsa | | | NOT RUN | |
| Nia | | | NOT RUN | |
| Libra H₂O | | | NOT RUN | |
| Forma | | | NOT RUN | |
| Clara HD | | | NOT RUN | |
| Aura H₂O Edition 2 (revision 2) | | | NOT RUN | |
| Aura Edition 2 (revision 2) | | | NOT RUN | |
| Tolino Vision Color | | | NOT RUN | |
| Tolino Shine Color | | | NOT RUN | |
| Tolino Shine (5th gen) | | | NOT RUN | |
| Aura ONE (experimental) | | | NOT RUN | |
| Glo HD (experimental) | | | NOT RUN | |
| Touch 2.0 (experimental) | | | NOT RUN | |
| Aura Edition 2 (revision 1, experimental) | | | NOT RUN | |
| Aura H₂O Edition 2 (revision 1, experimental) | | | NOT RUN | |
| Aura H₂O (experimental) | | | NOT RUN | |
| Aura (experimental) | | | NOT RUN | |
| Aura HD (experimental) | | | NOT RUN | |
| Glo (experimental) | | | NOT RUN | |
| Mini (experimental) | | | NOT RUN | |
| Touch A/B/C (experimental) | | | NOT RUN | |
