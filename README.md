# InoInk

An unofficial, standalone [Inoreader](https://www.inoreader.com) reader for the **Kobo Libra
Colour**, built on the [Plato](https://github.com/baskerville/plato) document reader. It talks to
your Inoreader account directly, keeps articles and their images for offline reading, and sends
your changes back when it can. There is no server, browser extension or read-later service in
between.

Project page with screenshots: <https://justmytwospence.github.io/InoInk/>

InoInk is **not affiliated with Inoreader or Rakuten Kobo**. It uses only Inoreader's documented
public API and does not try to match the Inoreader website feature for feature; see
[doc/INOINK-API.md](doc/INOINK-API.md).

## Features

- Three tabs, named as Inoreader names them: **Newsfeed** (unread by default), **Categories**
  (folders in your Inoreader order, by name or by most unread, then tags and the other streams)
  and **Read later** (your starred articles, the reading queue). Each view remembers its own
  filter, grouping and sort. Search covers every downloaded article.
- Tap a headline to read it; hold it to save it for later (star it). Settings can swap the two.
  At the end of an unread list a card offers **Mark N read**: it selects what you saw and did not
  save, for one tap on Read. Unread headlines you have not seen before carry a bar on the left;
  any list can switch to selection mode to mark or save many articles at once, with Undo.
- A compact list that uses the full width: bold for unread, the source, a relative date
  ("3 h ago") and the reading time ("6 min read") on one line, with the saved mark and the
  offline mark stacked at the right.
- You always know how far there is to go: lists show "2 / 7" and articles a bar along the
  bottom, split into one segment per page while they fit across the screen.
- Reading built for e-ink: Plato's typesetting with hyphenation and justification, and a
  Kobo-style **Aa** panel for font, size, line spacing, margins and alignment, which applies
  to every article.
- Math and diagrams: formulas from KaTeX, MathJax, MathML and plain LaTeX are typeset on the Kobo
  (sized with the text, offline once downloaded),
  and SVG images are shown. A formula
  that cannot be typeset appears as its LaTeX source.
- Hacker News and Lobsters: link posts open as the full linked article, with the discussion one
  tap away (an Article | comments switch in the bar). Both are cached for offline reading.
- **Done** marks an article read and removes its star in one tap, with Undo. **Mark read** keeps
  the star. Opening an article never marks it read.
- Reading flows: at the end of an article a card offers Done (or Mark read) and the next article
  of the list, Keep, Comments, or Back to list; swiping or pressing forward again continues (a
  setting). A line under the article shows progress, time left and your place in the list.
- Highlights and notes: hold a word to highlight it or add a note; Inoreader's own annotations
  are shown underlined. `inoink-cli highlights` exports yours as Markdown or Readwise-style JSON.
- Links: tap a link for a reading view of the page; hold it for Open in reading view, Show QR code
  (to open it on your phone), Send to Karakeep or Follow in Inoreader.
- Offline first: text and colour images are cached; every change is saved on the Kobo first and
  sent to Inoreader when the network is available.
- Light on your Inoreader API budget: a sync with nothing new costs one request, and Settings >
  Inoreader API budget shows today's usage against the limits Inoreader reports.
- Kobo conveniences: starts with the light and the Wi-Fi state the Kobo was using, a light panel
  (bulb icon) and a slide along the left edge for brightness, page-turn buttons, and a Wi-Fi icon
  in the top bar that is also the Wi-Fi switch.
- Tags, likes, broadcast, feed and folder management, and monitoring searches, as far as the
  public API allows.

## Status

Early and unofficial. It is developed and tested on one Kobo Libra Colour on firmware 4.46.23852,
but it is built to run on every device that runs Kobo firmware 4.6 or newer, including Tolinos:

- **Tested:** Kobo Libra Colour.
- **Expected to work, not yet tested** (Linux 4.x and newer, the same code paths): Clara Colour,
  Clara BW, Elipsa 2E, Clara 2E, Libra 2, Sage, Elipsa, Nia, Libra H₂O, Forma, Clara HD,
  Aura H₂O Edition 2 (revision 2), Aura Edition 2 (revision 2), and the Tolino Vision Color,
  Shine Color and Shine (5th generation) running Kobo firmware.
- **Experimental** (their Linux is older than the minimum the compiler targets, so InoInk may not
  start): Aura ONE, Glo HD, Touch 2.0, Aura Edition 2 (revision 1), Aura H₂O Edition 2
  (revision 1), Aura H₂O, Aura, Aura HD, Glo, Mini and Touch A/B/C. Main menu > Status says
  "(experimental)" on these.
- **Firmware 5.x:** experimental. Official NickelMenu does not run on it (a community build is in
  testing), so the launcher entry may be missing; without NickelMenu, start
  `.adds/inoink/current/inoink.sh` with any script launcher.

Other e-readers (Kindle, PocketBook, reMarkable, Android) are not supported: each would need its
own port. Reports from other devices are very welcome: open a **Device report** issue (the log
line `device:` in `.adds/inoink/data/inoink.log` and Main menu > Status > Device say what you have).
[doc/INOINK-ACCEPTANCE.md](doc/INOINK-ACCEPTANCE.md) says what has been checked, and how.

InoInk only touches its own folder on the Kobo (`.adds/inoink`) and, if you use NickelMenu, one
menu file (`.adds/nm/inoink`). Your books, settings and Kobo account are left alone. While it
runs, the normal Kobo interface is stopped; it comes back when you quit.

## What you need

1. A **Kobo** (or a Tolino running Kobo firmware; see Status below) with [NickelMenu](https://pgaskin.net/NickelMenu/) installed (it
   adds the menu entry that starts InoInk).
2. An **Inoreader account whose plan includes API access** (check Inoreader's developer page for
   current terms).
3. Your own **Inoreader app registration**: open
   <https://www.inoreader.com/developers/register-app>, choose the `read write` scope and set the
   redirect URI to `http://localhost:8080/callback`. Keep its App ID and App Key at hand.
   No key is shipped with InoInk.
4. A computer (macOS or Linux) for installing and signing in.

## Install

Download from the [latest release](https://github.com/justmytwospence/InoInk/releases/latest):

- `inoink-<version>-kobo.tar.gz`: the app
- `install-inoink.sh`: the installer
- `inoink-cli-<platform>`: the sign-in helper (or build it yourself, below)

Then:

1. Connect the Kobo by USB and tap **Connect** on the Kobo.
2. Install, with the menu entry:

   ```sh
   sh install-inoink.sh inoink-<version>-kobo.tar.gz /Volumes/KOBOeReader --menu
   ```

   Use your Kobo's mount point on Linux. The installer verifies every copied file and, on macOS,
   ejects the Kobo when it is done.
3. Unplug the Kobo, open the NickelMenu and choose **InoInk**. The first start creates the Kobo's
   own encryption key and shows "Sign in required". Quit InoInk from its main menu (≡, Quit).
4. Connect the Kobo again (tap **Connect**) and sign in on the computer:

   ```sh
   chmod +x inoink-cli-macos-arm64
   ./inoink-cli-macos-arm64 authorize --client-id YOUR_APP_ID \
       --redirect-uri http://localhost:8080/callback --device /Volumes/KOBOeReader
   ```

   It asks for your App Key at a hidden prompt, opens Inoreader in your browser for you to
   approve, and writes the sign-in to the Kobo **encrypted to that Kobo's key**.
5. Eject the Kobo and open InoInk again. It picks up the sign-in, connects using the Kobo's
   saved Wi-Fi network, and syncs.

To update, run step 2 again with the new package. Your data, sign-in and settings are kept, and
the previous version stays installed for rollback. To remove InoInk, run
`scripts/remove-inoink.sh /Volumes/KOBOeReader` from this repository (add `--purge-data` to
delete the cache and sign-in too).

More detail, including security notes and everyday use: [doc/INOINK-SETUP.md](doc/INOINK-SETUP.md).

## Using it

- **Tabs:** Newsfeed, Categories and Read later sit under the top bar. Tapping a headline opens
  it and holding saves it for later (Settings can swap them); long-press a row in Read later for
  Mark read, Done and Details. In Categories, the title of a folder or feed switches between the
  folder and its feeds.
- **Footer:** the label in the middle ("Unread · Newest") chooses unread or all, grouping and
  sort, and on the Categories list the order of folders. On the right, the page and how many
  there are ("2 / 7"; "+" when older articles can be loaded from Inoreader), with a bar above:
  one segment per page while they fit across the screen, otherwise a line; either way only the
  page shown is dark.
- **Sync:** InoInk syncs on its own when it starts, wakes or gets a network connection, and sends
  your changes a few seconds after you make them. Tap the sync icon to sync now; it shows a dot
  and turns while a sync runs. Main menu > Status shows the last sync, and Settings > Inoreader
  API budget today's requests.
- **Article:** tap the middle of the page for the bar (Back, Aa, Star, Done, More) and the progress
  line; tap the left or right third to turn pages. More: Mark read, Tags, Like, Broadcast, Send to
  Karakeep, Show QR code, Details, Load original page. Tap an image to enlarge it.
- **Light:** the bulb icon opens a panel for brightness and warmth; sliding along the left edge
  also changes brightness.
- **Wi-Fi:** the icon in the top bar shows off, connecting or online; tap it to switch Wi-Fi.
- **Karakeep:** `inoink-cli karakeep --url https://your-karakeep --device /Volumes/KOBOeReader`
  sets it up; see [doc/INOINK-SETUP.md](doc/INOINK-SETUP.md#karakeep).
- **USB:** quit InoInk before connecting to a computer.
- **Quitting:** on MediaTek models (Libra Colour, Clara Colour, Clara BW, Elipsa 2E), quitting restarts the Kobo (about half a minute). Restarting
  only the Kobo interface can crash these MediaTek models; see
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

## Contributing and releases

Changes go through pull requests to `main`, and each one is tried on a real Kobo before it is
merged. With the Kobo connected by USB, `scripts/kobo-dev.sh` builds your working tree and
installs it (your data on the Kobo is kept); the pull request template asks which device,
firmware and build you checked. Testing on other Kobo and Tolino models is especially welcome.

Merging is releasing: once CI passes on `main`, the Release workflow publishes the next version
to everyone. The version follows the pull request title: `feat:` bumps the minor version, a
breaking change (`feat!:`) the major, anything else the patch, and changes to docs, CI or tests
alone release nothing. `scripts/version.sh next` shows what merging now would publish. Details:
[doc/INOINK-SETUP.md](doc/INOINK-SETUP.md#development-and-releases).

## Documentation

| | |
|---|---|
| [doc/INOINK-SETUP.md](doc/INOINK-SETUP.md) | Install, sign in, update, remove, everyday use |
| [doc/INOINK-API.md](doc/INOINK-API.md) | What Inoreader's public API allows, and the limits |
| [doc/INOINK-SYNC.md](doc/INOINK-SYNC.md) | How changes are saved, sent and reconciled |
| [doc/INOINK-ACCEPTANCE.md](doc/INOINK-ACCEPTANCE.md) | What was tested: automated, emulator, device |
| [doc/UPSTREAM.md](doc/UPSTREAM.md) | Relationship to Plato |
| [doc/LICENSES.md](doc/LICENSES.md) | Licenses of everything that ships |
| [doc/plato/](doc/plato/README.md) | Plato's original documentation |

## Layout

- `crates/inoink`: the Inoreader library (database, API client, content pipeline, cache, sync).
  It has no user-interface dependency.
- `crates/inoink-cli`: the computer-side helper (`authorize`, `provision`, `fixtures`,
  `verify-cache`, `acceptance`).
- `crates/core`, `crates/plato`, `crates/emulator`: Plato's renderer, device layer and app,
  with the InoInk interface in `crates/core/src/view/inoink`.

## License

InoInk is a derivative of Plato, so the app as a whole is licensed under the
[GNU Affero General Public License v3.0 or later](LICENSE-AGPLv3), like Plato. You can use,
change and share it freely. If you distribute it, or run a modified version for others, you must
make your source code available under the same license.

The Inoreader crates written for InoInk (`crates/inoink` and `crates/inoink-cli`) are also
available under the [MIT License](LICENSE-MIT), so you can reuse them in other projects under
either license. Bundled fonts and libraries keep their own licenses; see
[THIRD-PARTY-NOTICES](THIRD-PARTY-NOTICES) and [doc/LICENSES.md](doc/LICENSES.md).

Plato is copyright Bastien Dejean and contributors.
