# InoInk

An unofficial, standalone [Inoreader](https://www.inoreader.com) reader for **Kobo
e-readers** (developed on a Kobo Libra Colour), built on the [Plato](https://github.com/baskerville/plato) document reader. It talks to
your Inoreader account directly, keeps articles and their images for offline reading, and sends
your changes back when it can. There is no server, browser extension or read-later service in
between.

Project page with screenshots, the device list, answers to common questions and an installer that
runs in Chrome or Edge: <https://justmytwospence.github.io/InoInk/>

InoInk is **not affiliated with Inoreader or Rakuten Kobo**. It uses only Inoreader's documented
public API and does not try to match the Inoreader website feature for feature; see
[doc/INOINK-API.md](doc/INOINK-API.md).

## Features

- Two tabs, one per Inoreader section. **Newsfeed** (unread by default) narrows to a folder, a
  feed or a monitoring feed; **Read later** (your saved articles, the reading queue) switches to
  the **Archive** (Inoreader's Archive: saved articles you finished and kept), Web pages,
  Annotations or a tag. The active tab names what you are looking at; tap it to choose. Each view
  remembers its own filter, grouping and sort. Search covers every downloaded article.
- Tap a headline to read it; tap the bookmark ribbon at its right to save it for later (star it);
  hold it for its menu (Save for later or Restore to Read later, Mark read, Archive or Remove from Archive, **Mark read up to here**, **Mark read
  from here down**, Details, Select). The mark-read button in the top bar asks, then marks the
  whole list read. At the end of an unread list a card offers **Mark N read**: one tap marks what
  you saw and did not save. Saved articles are never marked read by these, every one can be undone,
  and InoInk sends it and fetches new articles right away. Unread headlines you have not seen
  before carry a bar on the left; selection mode (the check-box button in the top bar) marks or
  saves many articles at once.
- Read later and the Archive show how far you got in each saved article ("Finished", "40% · 6 min left"),
  filters by progress (Not started, In progress, Finished), and sorts by when you saved it. It
  reads Inoreader's progress (undocumented, read-only) and adds your reading on the Kobo; an
  article you started on inoreader.com opens near where you stopped.
- A compact list that uses the full width: bold for unread, the source, a relative date
  ("3 h ago") and the reading time ("6 min read") on one line, with the saved mark and the
  offline mark stacked at the right.
- You always know how far there is to go: lists show "2 / 7" and articles a bar along the
  bottom, split into one segment per page while they fit across the screen.
- Reading built for e-ink: Plato's typesetting with hyphenation and justification, hanging
  quotes, small caps and underlines that skip descenders, and a
  Kobo-style **Aa** panel for font, size, line spacing, margins and alignment, which applies
  to every article. Dragging a slider reflows the page as you go, the font list shows each
  font in its own face, and text presets save a font, size, line spacing and alignment to
  switch between with one tap. Besides Libertinus Serif (the default) it offers fonts tuned for e-ink,
  Libron, NV Bitter, NV Jost and NV Literata from Nico Verbruggen's
  [eBook Fonts](https://github.com/nicoverbruggen/ebook-fonts), as bundled by
  [Kobalt](https://kobalt.nicoverbruggen.be), and any fonts you copy to the Kobo's `fonts` folder.
- Math and diagrams: formulas from KaTeX, MathJax, MathML and plain LaTeX are typeset on the Kobo
  (sized with the text, offline once downloaded),
  and SVG images are shown. A formula
  that cannot be typeset appears as its LaTeX source.
- Hacker News and Lobsters: link posts open as the full linked article, with the discussion one
  tap away (an Article | comments switch in the bar). Both are cached for offline reading. The comments are threaded for e-ink: a thin line down
  each reply branch, two indents at most (deeper replies name whom they answer), ⁂ and Skip ›
  between threads, and the thread you are in named in the top margin.
- **Archive** moves a saved article from Read later into Inoreader's Archive in one tap, with Undo,
  keeping its reading progress and leaving it read or unread as it was. Saving an archived article
  (**Restore to Read later**) moves it back. **Mark read** keeps the save. Opening an article never
  marks it read.
- Reading flows: paging past the end of an article opens a small popup: choose Mark read (the
  default), Archive (preselected on a saved article if you chose it last time) or Keep unread, see the next article's title, and tap Next article or swipe on
  to it, or tap Back to list. Swiping up or holding forward skips the popup. A line under the article shows progress,
  time left and your place in the list, with ‹ › to open the previous or next article.
- Highlights and notes: hold a word to highlight it or add a note, shaded, underlined or with a
  dotted underline, in colour on colour screens; Inoreader's own annotations are shown underlined.
  More > Export highlights writes yours as Markdown or HTML to `.adds/inoink/exports`, and
  `inoink-cli highlights` prints them as Markdown, HTML or Readwise-style JSON.
- Return after a jump: after following a footnote or a link inside the article, choosing a heading
  from More > Contents, or switching between article and comments, a Return button in the top
  margin takes you back.
- Images: tap one to enlarge it, then pinch or use the buttons to zoom, drag to pan and double-tap
  to switch between fit and enlarged; each image keeps its zoom while InoInk runs.
- Links: tap a link for a reading view of the page; hold it for Open in reading view, Show QR code
  (to open it on your phone), Send to Karakeep or Follow in Inoreader.
- Offline first: text and colour images are cached; every change is saved on the Kobo first and
  sent to Inoreader when the network is available.
- Light on your Inoreader API budget: a sync with nothing new costs one request, and Settings >
  Inoreader API budget shows today's usage against the limits Inoreader reports.
- Kobo conveniences: starts with the light and the Wi-Fi state the Kobo was using, a light panel
  (bulb icon) and a slide along the left edge for brightness, page-turn buttons, and a Wi-Fi icon
  in the top bar that is also the Wi-Fi switch.
- Sleep screens, after [Kobalt](https://kobalt.nicoverbruggen.be)'s: a status card with your
  unread and Read later counts, when InoInk last synced and the battery (the default); the
  page you were on, left as it was with a small Sleeping label; that page with a picture of
  yours over it; or the classic logo. Powering off shows the card in white on black.
- Tags, feed and folder management, and monitoring searches, as far as the
  public API allows.

## Status

Early and unofficial. It is developed and tested on one Kobo Libra Colour on firmware 4.46.23852,
but it is built to run on every device that runs Kobo firmware 4.6 or newer, including Tolinos:

- **Tested:** Kobo Libra Colour on firmware 4.46.23852.
- **Expected to work, not yet tested** (Linux 4.x and newer, the same code paths): Clara Colour,
  Clara BW, Elipsa 2E, Clara 2E, Libra 2, Sage, Elipsa, Nia, Libra H₂O, Forma, Clara HD,
  Aura H₂O Edition 2 (revision 2), Aura Edition 2 (revision 2), and the Tolino Vision Color,
  Shine Color and Shine (5th generation) running Kobo firmware.
- **Experimental** (their Linux is older than the minimum the compiler targets, so InoInk may not
  start): Aura ONE, Glo HD, Touch 2.0, Aura Edition 2 (revision 1), Aura H₂O Edition 2
  (revision 1), Aura H₂O, Aura, Aura HD, Glo, Mini and Touch A/B/C. Main menu > Status says
  "(experimental)" on these.
- **Firmware 5.x and newer:** experimental. Official NickelMenu does not run on it (a community
  build is in testing), and a firmware update there removes NickelMenu, so the launcher entry may
  be missing; without NickelMenu, start `.adds/inoink/current/inoink.sh` with any script
  launcher. [After a Kobo firmware update](doc/INOINK-SETUP.md#after-a-kobo-firmware-update)
  says how to get the entry back (NickelModManager can restore it for you).

InoInk checks itself when it starts. On a model or firmware that is not listed as tested, it
shows a page once naming them and asking for a Device report; it never refuses to start. If its
own files are damaged, it starts the previous version instead, or leaves the Kobo interface
running and shows "InoInk can't start" with what to do (reinstall); see
[Start-up check](doc/INOINK-SETUP.md#start-up-check).

Other e-readers (Kindle, PocketBook, reMarkable, Android) are not supported: each would need its
own port. Reports from other devices are very welcome: open a **Device report** issue (the log
line `device:` in `.adds/inoink/data/inoink.log` and Main menu > Status > Device say what you have).
[doc/INOINK-ACCEPTANCE.md](doc/INOINK-ACCEPTANCE.md) says what has been checked, and how. The
project page's device list and installer use the same groups (`site/kobo.js`; a test keeps the two
in step).

InoInk only touches its own folder on the Kobo (`.adds/inoink`) and, if you use NickelMenu, one
menu file (`.adds/nm/inoink`). Your books, settings and Kobo account are left alone. While it
runs, the normal Kobo interface is stopped; it comes back when you quit.

## What you need

1. A **Kobo** (or a Tolino running Kobo firmware; see Status below) with [NickelMenu](https://pgaskin.net/NickelMenu/) installed (it
   adds the menu entry that starts InoInk).
2. An **Inoreader account whose plan includes API access** (check Inoreader's developer page for
   current terms).
3. Your own **Inoreader app registration** (no key is shipped with InoInk). In Inoreader, open
   [Preferences > Developer API](https://www.inoreader.com/preferences/other), click
   **New application** and fill in:
   - **Name:** `InoInk` (anything you like)
   - **Platform:** Other
   - **Redirect URI:** `https://justmytwospence.github.io/InoInk/callback.html` (a page on the
     project site that only hands Inoreader's answer back to your computer or Kobo)
   - **OAuth scope:** Read and write

   Save, and keep the **App ID** and **App key** it shows at hand. If New application is greyed
   out, your account already has an application: you can use that one instead (InoInk then shares
   its daily request budget); when its redirect URI is something else, you paste the address the
   browser ends on into the terminal after approving.
4. A Mac or Linux computer with a terminal, or Chrome or Edge for the browser installer.

## Install

Connect the Kobo by USB, tap **Connect** on the Kobo, and run in a terminal:

```sh
curl -fsSL https://justmytwospence.github.io/InoInk/install.sh | sh
```

It downloads the latest release and checks it against the published checksums, installs it on the
Kobo with the NickelMenu entry, asks for your App ID and App key, opens Inoreader for you to
approve, offers to set up [Karakeep](doc/INOINK-SETUP.md#karakeep) (Enter skips it) and ejects the
Kobo. Unplug it and open **InoInk** from NickelMenu: it finishes installing, picks up your accounts
and syncs.

- To read the script first: download `install.sh` from the [project page](https://justmytwospence.github.io/InoInk/),
  read it, then run `sh install.sh`.
- Options go after `sh -s --`: `--device /path/to/KOBOeReader`, `--version X.Y.Z`,
  `--accounts-only` (skip the install: sign in again or change accounts) and `--service karakeep`
  (only that service).
- **In Chrome or Edge instead:** on the [project page](https://justmytwospence.github.io/InoInk/#installer),
  choose **Install or remove on Kobo**, then the KOBOeReader drive. The page shows your model,
  firmware and what is installed, downloads the package from the project site, checks it against
  its checksum and writes the same files as the script; it can also save your Karakeep address and
  key. Then sign in on the Kobo: Main menu > Settings > Accounts > **Set up from a phone** shows a
  QR code for a page the Kobo serves on your Wi-Fi. Without NickelMenu, install it first with
  [KoboPatch Web UI](https://kp.nicoverbruggen.be).
- **By hand:** download `install-inoink.sh`, the Kobo package and `inoink-cli-<platform>` from the
  [latest release](https://github.com/justmytwospence/InoInk/releases/latest), then
  `sh install-inoink.sh inoink-<version>-kobo.tar.gz /Volumes/KOBOeReader --menu` and
  `./inoink-cli-macos-arm64 setup --device /Volumes/KOBOeReader` (on macOS clear the quarantine
  flag of a browser-downloaded binary first: `xattr -d com.apple.quarantine inoink-cli-macos-arm64`).

Signing in cannot happen on the project page itself: Inoreader's token endpoint sends no CORS
headers, so a page cannot read its answer without a server in between that would see your App key
and tokens.

### First five minutes

- You land on **Newsfeed**, your unread articles. **Tap** a headline to read it, tap its
  **ribbon** at the right to save it for later, **hold** it for its menu.
- In an article, turn pages by tapping the left or right third (or the page-turn buttons). Tap
  the middle for the bar: tap the article's title (or the arrow) to return to the list,
  **Archive** moves the article to your Inoreader Archive, **Aa** changes the font and spacing.
  Paging past the last page opens a small popup to mark it read, archive it or keep it unread.
- Tap the active tab to choose a folder (from **Newsfeed**) or the Archive, Web pages, Annotations or a tag (from **Read later**).
- To clear a list: the check-mark button in the top bar marks it all read (after asking), a
  held headline's menu offers **Mark read up to here** and **from here down**, and at the end of
  an unread list **Mark N read** clears what you skimmed in one tap. Saved articles stay unread.
- Changes go to Inoreader on their own when Wi-Fi is on. To quit, use the main menu (≡ > Quit),
  and always quit before plugging the Kobo into a computer.

To update, run the same command again. Your data, sign-in and settings are kept, and
the previous version stays installed for rollback. Installing also brings an existing menu entry up
to date. To remove InoInk, use **Remove InoInk** in the
browser installer, or run `scripts/remove-inoink.sh /Volumes/KOBOeReader` from this repository.
Both keep your data by default; `--reset-settings` (Reset settings) also deletes the settings,
and `--purge-data` (Delete everything) deletes the whole `.adds/inoink` folder, sign-in and
highlights included.

More detail, including security notes and everyday use: [doc/INOINK-SETUP.md](doc/INOINK-SETUP.md).

## Using it

- **Tabs:** Newsfeed and Read later sit under the top bar. Tapping the other tab opens it; tapping the active one opens its list of views. Tapping a headline opens
  it, tapping the ribbon at its right saves it for later, and holding shows its menu (Mark read,
  Archive, Mark read up to here / from here down, Details, Select). Inside a folder, **Feeds ▾** in the top bar switches between the
  folder and its feeds.
- **Footer:** three labels, each with its own short menu: what is shown ("Unread ▾"), the
  grouping ("No grouping ▾") and the sort ("Newest ▾"); on the folder list, one label for the
  order of folders. On the right, the page and how many
  there are ("2 / 7"; "+" when older articles can be loaded from Inoreader), and above the footer,
  between it and the headlines, the page indicator: one segment per page while they fit across the screen, otherwise a line; either way only the
  page shown is dark.
- **Sync:** InoInk syncs on its own when it starts, wakes or gets a network connection, and sends
  your changes a few seconds after you make them. While the Wi-Fi is off a sync waits for it, or,
  with Settings > Sync without Wi-Fi > Turn on Wi-Fi to sync, turns it on and off again
  afterwards. After 3 failed syncs in a row it waits for a wake or your tap. Tap the sync icon to sync now; it shows a dot
  and turns while a sync runs. the main menu shows when the last sync was ("Synced 5 min ago"), and Settings > Inoreader
  API budget today's requests.
- **Article:** tap the middle of the page for the bar (the arrow and the title go back; Aa, Star, Archive, More) and the progress
  line; tap the left or right third to turn pages. More: Mark read, Tags, Send to
  Karakeep, Show QR code, Export highlights, Contents (articles with headings), Details, Load original page (the article
  switches to it when it arrives). Tap an image to enlarge it. After a jump, Return in the top margin goes back; hold it to stay.
- **Light:** the bulb icon opens a panel for brightness and warmth; sliding along the left edge
  also changes brightness.
- **Wi-Fi:** the icon in the top bar shows off, connecting or online; tap it to switch Wi-Fi.
- **Accounts:** Main menu > Settings > Accounts shows Inoreader and Karakeep with Test and Sign out
  (Remove for Karakeep), and **Set up from a phone** shows a QR code for a page on the Kobo that
  signs in from a phone on the same Wi-Fi. See [doc/INOINK-SETUP.md](doc/INOINK-SETUP.md#accounts-on-the-kobo).
- **Karakeep:** set it up while installing, in Settings > Accounts, or later with
  `curl -fsSL https://justmytwospence.github.io/InoInk/install.sh | sh -s -- --accounts-only --service karakeep`;
  see [doc/INOINK-SETUP.md](doc/INOINK-SETUP.md#karakeep).
- **Messages:** short notes appear one at a time, centred below the top bar. Archive and bulk
  changes update the list without one; Main menu > Undo last change reverts them.
- **USB:** quit InoInk before connecting to a computer.
- **Sleep screen:** Main menu > Settings > Sleep screen chooses Status card, Current page,
  Picture over the page or Logo. For pictures, copy PNG files into `.adds/inoink/screensaver`
  on the Kobo drive: one is picked at random each time, scaled to fit the screen, and
  transparent parts let the page show through (until there is a PNG there, the page shows on its
  own). The screen is drawn from what is already on the Kobo: going to sleep never waits for the
  network. See [doc/INOINK-SETUP.md](doc/INOINK-SETUP.md#sleep-screen).
- **Quitting:** on MediaTek models (Libra Colour, Clara Colour, Clara BW, Elipsa 2E), quitting after the Wi-Fi was used restarts the Kobo (about half a minute), because restarting
  only the Kobo interface can then crash it. With the Wi-Fi off from start to quit, it returns to
  the Kobo interface directly (so not with Turn on Wi-Fi to sync, which uses the Wi-Fi at launch). The menu says which: Quit, or Quit (the Kobo restarts). See
  [doc/INOINK-SETUP.md](doc/INOINK-SETUP.md#quitting).

## Build from source

You need Docker, `curl` and `jq` for the Kobo build, and Rust (see `rust-toolchain.toml`) for
the sign-in helper.

```sh
scripts/fetch-inputs.sh                  # pinned, checksummed toolchain and library sources
scripts/build-inoink.sh --target kobo    # cross-compile in a container
scripts/package-inoink.sh                # dist/inoink-<version>-kobo.tar.gz and source archives
cargo build --release -p inoink-cli      # target/release/inoink-cli
scripts/test-inoink.sh                   # all automated checks
```

Builds made this way are named as dev versions (`<last release>-dev.<commits>.<commit>`); only
the Release workflow builds release versions.

`scripts/build-inoink.sh --run-emulator --data target/inoink-fixtures` runs a scripted tour in
the desktop emulator and writes screenshots to `target/inoink-emulator-shots`. Create the sample
data first with `cargo run -p inoink-cli -- fixtures --output target/inoink-fixtures`.

To review UI changes screen by screen, `scripts/preview-macos.sh --capture` saves the tour's
screens, and `node scripts/review/review.mjs` turns them into rounds on a local page where notes
are pinned to places on a screen, with before and after side by side
([doc/INOINK-SCREEN-REVIEW.md](doc/INOINK-SCREEN-REVIEW.md)). To time a release on a real Kobo
(page turns, opening the list and articles, memory, a sync with nothing new), see
[doc/INOINK-BENCHMARKS.md](doc/INOINK-BENCHMARKS.md).

### Interactive desktop preview (Apple Silicon Mac)

With Homebrew, Rust (see `rust-toolchain.toml`) and Xcode command-line tools installed, run:

```sh
scripts/preview-macos.sh
```

The first run installs missing Homebrew libraries and builds the pinned MuPDF renderer; later
runs reuse those libraries and incrementally rebuild your Rust changes. No Docker or Kobo is
needed. The native window shows the UI at half size (632x840), using coverage averaging to retain
fine rules when reduced. Layout and saved screenshots retain the Libra Colour's full 1264x1680
device-resolution pixels.

By default the preview uses offline sample articles, not your Inoreader account. Changes and
settings persist in `target/macos-preview/state`; build caches stay separately under
`target/macos-preview`.

```sh
scripts/preview-macos.sh --reset       # reset only sample data, settings and screenshots
scripts/preview-macos.sh --scale 0.4   # use a smaller window
scripts/preview-macos.sh --build-only  # prepare/build without opening a window
scripts/preview-macos.sh --inoreader   # use a sandbox snapshot of your real articles
scripts/preview-macos.sh --inoreader --refresh  # replace local experiments with a fresh snapshot
scripts/preview-macos.sh --live        # your real account, syncing (changes reach Inoreader)
scripts/preview-macos.sh --capture     # the scripted tour without a window, for screen review
```

The Inoreader mode asks for a **dedicated read-only authorization** on first use; a separate
read-only application is recommended. It downloads up to 100 recent unread and 100 saved
articles, with subscriptions and folders, to `target/macos-preview/inoreader/state`. Downloading
uses Inoreader API quota and may request article pages and images from publishers. Counts can
reflect your whole account even though only a subset of articles is cached. This is not a full
account export.

After download, the preview runs offline: marking articles read, starring, archiving and Undo
change **only the local sandbox**. Relaunching keeps those experiments; `--refresh` replaces
them with a new snapshot. Credentials are stored separately under
`target/macos-preview/inoreader/auth`, never inside the preview data. See
[setup](doc/INOINK-SETUP.md#real-article-desktop-sandbox) for authorization and security details.

Click, hold and drag to tap, long-press and swipe. **B/F** simulate page buttons, **[ / ]** rotate,
**S** saves a full-resolution PNG in the selected preview's `state/screenshots`, and **Escape** or
closing the window quits. Relaunch after editing code to try the new build. The preview does not
simulate e-ink ghosting, refresh timing or physical hardware; device acceptance before merging
still applies. Only one preview/build runs at a time. After a forced kill, a stale lock is left
untouched: confirm no preview or build is running before manually removing the lock directory
printed by the launcher.

## Contributing and releases

Changes go through pull requests to `main`, and each one is tried on a real Kobo before it is
merged. With the Kobo connected by USB, `scripts/kobo-dev.sh` builds your working tree and
installs it (your data on the Kobo is kept); the pull request template asks which device,
firmware and build you checked. Testing on other Kobo and Tolino models is especially welcome.

Merging is releasing: once CI passes on `main`, the Release workflow publishes the next version
to everyone. The version follows the pull request title: `feat:` bumps the minor version, a
breaking change (`feat!:`) the major, anything else the patch, and changes to docs, CI, tests or
development tools alone release nothing. `scripts/version.sh next` shows what merging now would publish. Details:
[doc/INOINK-SETUP.md](doc/INOINK-SETUP.md#development-and-releases).

## Documentation

| | |
|---|---|
| [doc/INOINK-SETUP.md](doc/INOINK-SETUP.md) | Install, sign in, update, remove, everyday use |
| [doc/INOINK-API.md](doc/INOINK-API.md) | What Inoreader's public API allows, and the limits |
| [doc/INOINK-SYNC.md](doc/INOINK-SYNC.md) | How changes are saved, sent and reconciled |
| [doc/INOINK-POWER.md](doc/INOINK-POWER.md) | Timers, Wi-Fi rules and storage writes: what costs power |
| [doc/INOINK-ACCEPTANCE.md](doc/INOINK-ACCEPTANCE.md) | What was tested: automated, emulator, device |
| [doc/INOINK-SCREEN-REVIEW.md](doc/INOINK-SCREEN-REVIEW.md) | Reviewing screens: rounds, pinned notes, before and after |
| [doc/INOINK-BENCHMARKS.md](doc/INOINK-BENCHMARKS.md) | Timing a release on a real Kobo |
| [doc/UPSTREAM.md](doc/UPSTREAM.md) | Relationship to Plato |
| [doc/LICENSES.md](doc/LICENSES.md) | Licenses of everything that ships |
| [doc/plato/](doc/plato/README.md) | Plato's original documentation |

## Layout

- `crates/inoink`: the Inoreader library (database, API client, content pipeline, cache, sync).
  It has no user-interface dependency.
- `crates/inoink-cli`: the computer-side helper (`setup`, `authorize`, `karakeep`,
  `highlights`, `provision`, `fixtures`, `verify-cache`, `acceptance`, and `snapshot-login` /
  `snapshot` for the desktop preview).
- `crates/core`, `crates/plato`, `crates/emulator`: Plato's renderer, device layer and app,
  with the InoInk interface in `crates/core/src/view/inoink`.
- `crates/inoink-bench`: the on-device benchmark (its own Cargo workspace, standard library
  only); `scripts/review`: the screen review page. Development tools; neither ships.

## License

InoInk is a derivative of Plato, so the app as a whole is licensed under the
[GNU Affero General Public License v3.0 or later](LICENSE-AGPLv3), like Plato. You can use,
change and share it freely. If you distribute it, or run a modified version for others, you must
make your source code available under the same license.

The Inoreader crates written for InoInk (`crates/inoink` and `crates/inoink-cli`) are also
available under the [MIT License](LICENSE-MIT), so you can reuse them in other projects under
either license. Bundled fonts and libraries keep their own licenses; see
[THIRD-PARTY-NOTICES](THIRD-PARTY-NOTICES) and [doc/LICENSES.md](doc/LICENSES.md).

Plato is copyright Bastien Dejean and contributors. Return after a jump, the image viewer,
highlight styles and the Aa panel's live reflow and text presets follow ideas from
[Kobalt](https://github.com/nicoverbruggen/kobalt) (MIT); no Kobalt code is included.
