# Setup, update and removal

## What you need

- A Kobo Libra Colour. InoInk replaces the normal Kobo interface while it runs and restores it
  when you quit. Firmware 4.46.23852 is the target; compatibility is only established by the
  device tests in `doc/INOINK-ACCEPTANCE.md`.
- [NickelMenu](https://pgaskin.net/NickelMenu/) (a launcher entry) is optional but recommended.
  Without it, start `.adds/inoink/current/inoink.sh` any way you launch scripts.
- An Inoreader account with API access. Inoreader's developer portal states that the API is for
  public apps on unsupported platforms and otherwise needs a Pro plan; confirm your plan allows
  it. Requests are limited per day in two zones (reads and writes); the limits depend on your
  plan. InoInk reads the real limits from the server's response headers and shows them in
  Settings > Inoreader API budget.
- A registered Inoreader application (https://www.inoreader.com/developers/register-app) with a
  redirect URI such as `http://localhost:8080/callback` and the `read write` scope. Its client
  id and secret are yours: **no secret is shipped in InoInk**.

## Build

```sh
scripts/fetch-inputs.sh                 # downloads and verifies the pinned toolchain and sources
scripts/build-inoink.sh --target kobo   # cross-build (Linaro GCC 4.9.4) in a container
scripts/package-inoink.sh               # dist/inoink-<version>-kobo.tar.gz + source archives
```

Requirements on the computer: Docker, `curl`, `jq`. `scripts/build-inoink.sh --target desktop`
builds the emulator; `scripts/build-inoink.sh --run-emulator --data target/inoink-fixtures`
replays a scripted tour and writes screenshots to `target/inoink-emulator-shots`.

## Development and releases

1. Work on a branch and open a pull request against `main`. Its title is the commit message
   (conventional commits: `feat:`, `fix:`, `perf:`, `docs:` ...) and decides the release below.
2. Try it on a Kobo: connect it by USB and run `scripts/kobo-dev.sh` (add `--menu` the first
   time). It builds the working tree, uncommitted changes included, as a dev version such as
   `0.2.3-dev.4.1a2b3c4.dirty` (Main menu > Status shows it), installs it and ejects the Kobo; your
   data is kept. Without Docker, or with `--ci`, it installs the package CI built for the pushed
   HEAD instead. To go back, install a release the same way, or write the previous release's name
   into `.adds/inoink/active-release`.
3. Note in the pull request what you checked, on which device and firmware, and with which build.
   Cases in `doc/INOINK-ACCEPTANCE.md` move from NOT RUN to PASS only with device evidence.
4. CI must pass (`test`, `core`, `kobo`); the `kobo` job also uploads the package it built.
5. Squash-merge. Merging is releasing: once CI passes on `main`, the Release workflow tags and
   publishes the next version to everyone, with the Kobo package, the installer and `inoink-cli`.
   The version comes from the titles merged since the last release: `feat:` bumps the minor
   version, `type!:` or `BREAKING CHANGE:` the major, anything else the patch; changes that only
   touch docs, CI or tests release nothing. `scripts/version.sh next` shows what merging now would
   publish. Versions are never bumped or tagged by hand.

## Install

1. Connect the Kobo by USB and open its storage.
2. Install: `scripts/install-inoink.sh dist/inoink-<version>-kobo.tar.gz /path/to/KOBOeReader --menu`
   (`--menu` adds the NickelMenu entry when NickelMenu is installed).
3. Eject the Kobo, open the NickelMenu entry "InoInk". The computer only copies the package
   (`.adds/inoink/INSTALL.TGZ` and its checksum); InoInk unpacks and activates it when it starts,
   because folders and long file names created from macOS get renamed to `FSCK0000.000` by the
   Kobo's disk check. `data/inoink.log` says "installed release <version>". The first launch also
   creates the device key.
4. Quit InoInk (main menu, Quit), reconnect the Kobo, then sign in **on the computer**:

```sh
# inoink-cli from the release, or: cargo build --release -p inoink-cli
inoink-cli authorize \
    --client-id YOUR_ID --redirect-uri http://localhost:8080/callback \
    --device /path/to/KOBOeReader
```

   Approve access in the browser. The helper exchanges the code, learns your account id and writes
   the credentials **encrypted to the Kobo's own key** (age X25519) into
   `.adds/inoink/data/provision/`. Eject and open InoInk: it imports and deletes the transfer
   file, then syncs. Tokens refresh on the device; the Kobo's browser is never needed.

Security note: the client secret has to live on the Kobo because Inoreader's documented token
refresh needs it. The Kobo's storage is FAT with no file permissions, so anyone with the device
can read it. Use a dedicated application registration you can revoke.

## Quitting

On MediaTek Kobos (Libra Colour, Clara Colour, Clara BW), quitting InoInk **reboots** the Kobo
instead of restarting its interface directly. Restarting the interface after another app has used
the Wi-Fi chip can crash the kernel (koreader/koreader#12739); the device then reboots uncleanly
and may show its setup screens. A clean reboot takes about half a minute and avoids that. To
restart the interface directly anyway, put `INOINK_RESTART_NICKEL=1` in
`.adds/inoink/data/config.sh`.

## Update

Run the install command again with the new package. Only `.adds/inoink/releases` and the small
`active-release` manifest change; `.adds/inoink/data` (database, cache, credentials, settings) is
never modified, and the database migrates itself on first start (after taking a backup next to
the database file). The previous release stays installed for rollback: write its name into
`.adds/inoink/active-release`.

## Remove

```sh
scripts/remove-inoink.sh /path/to/KOBOeReader               # keeps your data
scripts/remove-inoink.sh /path/to/KOBOeReader --purge-data  # also deletes cache and credentials
```

## Everyday use

- Three tabs sit under the top bar. **Newsfeed** shows unread articles from everything you follow;
  **Categories** lists your folders in Inoreader's order (with the unread count on the device),
  then tags and other streams (Saved pages, Annotated, Liked, Broadcast, monitoring searches);
  **Read later** is your starred articles. The numbers in the tabs are counts on the device.
- In the Newsfeed and in categories, **tap a headline to read it** and **hold it to save it for
  later** (it gets a bookmark at the right of the title and a bar along the left edge). Main menu > Settings > Headlines swaps
  these. In Read later, holding a headline shows its menu.
- The top bar keeps what you only read (clock, battery) on the left and every button on the right:
  search, mark this list read (circled check mark; shows a count first), sync, Wi-Fi, light, menu.
  The sync icon shows a dot and turns while a sync runs; Main menu > Status shows the last sync.
- InoInk syncs when it starts, wakes or gets a network connection (at most every 15 minutes), and
  sends your changes a few seconds after you make them; there is no timer keeping the Kobo awake.
  A sync with nothing new costs one Inoreader request. Settings > Inoreader API budget shows
  today's reads and writes against the limits Inoreader reports, when they reset and how many
  requests the last sync used.
- Math and diagrams: formulas from KaTeX, MathJax, MathML and plain LaTeX are typeset on the Kobo (sized with the text, offline once downloaded), and SVG images are shown. A formula that cannot be typeset appears as its LaTeX source. Formulas appear in articles downloaded after updating; Load original page re-downloads one. Tapping a formula enlarges it.
- Swiping back on the first page of an article opens the previous article of the list at its
  beginning.
- A thin bar along the bottom of an article shows where you are: one segment per page, dark up to the
  page shown, while each can be at least 6 mm wide (so landscape fits more than portrait, up to 20), otherwise a line.
- On the last page of an unread list a card says "Mark N read": N is what you were shown in this
  visit that is still unread and not starred. Nothing is marked read by turning pages.
- The footer label ("Unread · Newest") opens a menu: unread only or all, group (none, unread
  first, by source) and sort (newest, oldest, shortest, longest, title, source). Each view
  remembers its own choice; defaults are unread only for the Newsfeed, everything for Read
  later, and everything with unread first for folders and feeds. Loading older articles from
  Inoreader at the end of a list only works with all articles, no grouping and newest first.
  On the Categories list the footer label sorts folders: Inoreader's order, name, or most unread.
- The footer shows the page and how many there are ("2 / 7"; "+" when older articles can be
  loaded from Inoreader), and the line above the footer is the page indicator: one segment per page with the page shown
  dark (a short dark stretch on the line when there are too many pages for segments). Swipe or use the page
  buttons to move between pages of a list. In Categories, the title of a folder or feed opens a menu to switch between the
  folder and its feeds; the arrow at the top left goes back to the list of categories.
- In an article, tap the left or right third of the page to turn pages and the middle for the bar:
  Back, Aa, Star, Done, More, with a line below showing progress, time left and your place in the
  list. Aa opens the text panel (font, size, line spacing, margins, alignment); its settings apply
  to every article. More holds Mark read/unread, Tags, Like, Broadcast, Send to Karakeep, Show
  QR code, Details (including summaries, your highlights and annotations returned by Inoreader) and Load original
  page.
- Swipe left or right to turn pages. Swipe up, page past the last page or hold the forward button
  for the next article of the list; swipe down or hold the back button for the previous one (swipe directions follow Settings >
  Page-turn buttons: Inverted swaps them; it reopens where you left it; paging back from the first page opens its last page). Changing
  articles never marks one read: use Done or Mark read. After the last article of the list you are
  back at the list. With the bar showing, a row above the footer names the previous (left) and next
  (right) articles; tap one to open it. The article's title stays in the top margin of every page after the first. Straight quotes,
  `--` and `...` in articles are set as curly quotes, dashes and ellipses, and lines are hyphenated
  in the article's language when InoInk can tell it (English otherwise). The previous and next articles download in
  the background while you read. Settings > Previous and next titles > Always also names them beside
  the progress line on every page.
- Articles whose feed gives only a teaser (short text or a "Read more" link) are replaced by the full
  page when they are downloaded. InoInk learns which feeds do this; Manage > a feed > Full text sets
  Always or Never for one feed.
- Highlights: hold a word in an article, drag to extend, then choose Highlight or Add Note. Hold
  a highlight to add, edit or remove its note, or to remove it. They are saved on the Kobo and
  found again if the article text is later replaced (for example by Load original page). Inoreader's
  own annotations are underlined and read-only; hold one to see its note. Details lists both.
  `inoink-cli highlights --data /Volumes/KOBOeReader/.adds/inoink/data` prints yours as Markdown
  (`--format json` gives Readwise's highlight fields; nothing is sent anywhere).
- Links in an article: tap one for a reading view of the page (InoInk fetches it and extracts the
  text, without images; https only, needs Wi-Fi). Hold one for a menu: Open in reading view, Show
  QR code (scan it with a phone to open the page there), Send to Karakeep, Follow in Inoreader
  (adds the site or feed to your subscriptions).
- Items from Hacker News and Lobsters feeds are link posts: InoInk downloads the linked page and
  extracts its text itself (no credentials, not an Inoreader feature), and caches the thread from
  the site's public JSON. The bar shows an Article | N comments switch (also a link at the end
  of the article); More has Refresh comments. Star, Done and Read apply to the one item.
  Pages that cannot be extracted (paywalls, scripts, PDFs) fall back to the feed text with a note.
- Tap an image to enlarge it; tap again to zoom, swipe to pan. A grey "Image unavailable - retry"
  line is a retryable placeholder.
- After **Done**, an Undo bar sits at the top of the list until you undo it or finish another article.
- Main menu: Follow a feed, Settings (what a tap does, where the previous and next titles show, offline
  storage, how long to keep unread articles, page-turn buttons, rotation, invert colors,
  Inoreader API budget), Manage (rename or move feeds between folders, unfollow, rename or delete folders, keep a
  folder offline, monitoring searches), Status (sync, changes waiting, Inoreader's daily limits,
  Karakeep) and About.
- The bulb icon opens the light panel: on or off, brightness and warmth. Sliding a finger up or
  down the left edge also changes brightness. InoInk starts with the light and the Wi-Fi state
  the Kobo had (the launcher writes what it found to `inoink.log`, on the line starting `launch:`).
- The Wi-Fi icon in the top bar is the only Wi-Fi control: tap it to switch Wi-Fi on or off (a
  dashed icon means switched on but not connected yet). InoInk uses the networks the Kobo
  already knows.
- Quit InoInk before connecting the Kobo to a computer. Suspend stops network work before the
  device sleeps.

## Karakeep

Links and articles can be sent to [Karakeep](https://karakeep.app) (More menu, or by holding a
link). They wait in a local queue and are sent as soon as the network allows; Karakeep's address
and API key are set up from the computer, the same way as the Inoreader sign-in:

```sh
inoink-cli karakeep --url https://karakeep.example.com --device /Volumes/KOBOeReader
```

The API key (create one in Karakeep under Settings > API Keys) is read from the
`INOINK_KARAKEEP_KEY` environment variable or a hidden prompt, never from the command line. The
helper checks it against your Karakeep, then writes it encrypted to the Kobo's own key; InoInk
imports and deletes the transfer file at the next start. Like the Inoreader client secret, the key
then lives on the Kobo's FAT storage, which has no file permissions: use a key you can revoke.
Main menu > Status shows how many links are waiting or failed.
