# InoInk

An unofficial, standalone [Inoreader](https://www.inoreader.com) reader for the **Kobo Libra
Colour**, built on the [Plato](https://github.com/baskerville/plato) document reader. It talks to
your Inoreader account directly, keeps articles and their images for offline reading, and sends
your changes back when it can. There is no server, browser extension or read-later service in
between.

InoInk is **not affiliated with Inoreader or Rakuten Kobo**. It uses only Inoreader's documented
public API and does not try to match the Inoreader website feature for feature; see
[doc/INOINK-API.md](doc/INOINK-API.md).

## Features

- Starts on your Starred articles (then the last collection you used). Unread/All filter,
  search over downloaded articles, and a drawer with folders, feeds and tags.
- A compact list: bold for unread, the source, a relative date ("3 h ago") and the reading
  time ("6 min read") on one line.
- Reading built for e-ink: Plato's typesetting with hyphenation and justification, and a
  Kobo-style **Aa** panel for font, size, line spacing, margins and alignment, which applies
  to every article.
- Hacker News and Lobsters: link posts open as the full linked article, with the discussion one
  tap away (an Article | comments switch in the bar). Both are cached for offline reading.
- **Done** marks an article read and removes its star in one tap, with Undo. **Mark read** keeps
  the star. Opening an article never marks it read.
- Offline first: text and colour images are cached; every change is saved on the Kobo first and
  sent to Inoreader when the network is available.
- Kobo conveniences: starts with the brightness and warmth the Kobo was using, slide along the
  left edge to change brightness, page-turn buttons, and a Wi-Fi indicator in the top bar.
- Tags, likes, broadcast, feed and folder management, and monitoring searches, as far as the
  public API allows.

## Status

Early and unofficial (version 0.1.0). It is developed and tested on one Kobo Libra Colour on
firmware 4.46.23852. Other Kobo models that Plato supports may work, but nobody has tested them.
[doc/INOINK-ACCEPTANCE.md](doc/INOINK-ACCEPTANCE.md) says what has been checked, and how.

InoInk only touches its own folder on the Kobo (`.adds/inoink`) and, if you use NickelMenu, one
menu file (`.adds/nm/inoink`). Your books, settings and Kobo account are left alone. While it
runs, the normal Kobo interface is stopped; it comes back when you quit.

## What you need

1. A **Kobo Libra Colour** with [NickelMenu](https://pgaskin.net/NickelMenu/) installed (it
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
   sh install-inoink.sh inoink-0.1.0-kobo.tar.gz /Volumes/KOBOeReader --menu
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

- **List:** tap an article to read it. Tap the title for collections, use the footer for
  Unread/All, the group and sort menu (per view: Starred puts unread first, folders group by
  source; sort by newest, oldest, shortest, longest, title or source) and sync status, and long-press a row for Star, Mark read, Done and Details.
- **Article:** tap the middle of the page for the bar: Back, **Aa** (text settings), Star, Done
  and More (Mark read, Tags, Like, Broadcast, Details, Load original page). Tap an image to
  enlarge it.
- **Light:** slide a finger up or down along the left edge of the screen.
- **Wi-Fi:** the icon in the top bar shows the connection; tap it to switch Wi-Fi on or off.
- **USB:** quit InoInk before connecting to a computer.

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

`scripts/build-inoink.sh --run-emulator --data target/inoink-fixtures` runs a scripted tour in
the desktop emulator and writes screenshots to `target/inoink-emulator-shots`. Create the sample
data first with `cargo run -p inoink-cli -- fixtures --output target/inoink-fixtures`.

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
