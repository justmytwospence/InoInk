# Setup, update and removal

## What you need

- A Kobo Libra Colour. InoInk replaces the normal Kobo interface while it runs and restores it
  when you quit. Firmware 4.46.23852 is the target; compatibility is only established by the
  device tests in `doc/INOINK-ACCEPTANCE.md`.
- [NickelMenu](https://pgaskin.net/NickelMenu/) (a launcher entry) is optional but recommended.
  Without it, start `.adds/inoink/current/inoink.sh` any way you launch scripts.
- An Inoreader account with API access. Inoreader's developer portal states that the API is for
  public apps on unsupported platforms and otherwise needs a Pro plan; confirm your plan allows
  it. The default Pro limit is 100 requests per day in each of two zones (read and write);
  InoInk reads the real limits from the server's response headers.
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

## Install

1. Connect the Kobo by USB and open its storage.
2. Install: `scripts/install-inoink.sh dist/inoink-<version>-kobo.tar.gz /path/to/KOBOeReader --menu`
   (`--menu` adds the NickelMenu entry when NickelMenu is installed).
3. Eject the Kobo, open the NickelMenu entry "InoInk". The first launch creates the device key.
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

- The footer label next to Unread/All (for example "Newest ▾") opens the group and sort menu. Group:
  none, unread first, or by source. Sort: newest, oldest, shortest, longest, title or source.
  Each view remembers its own choice; defaults are unread first for Starred, by source for
  folders, and newest first elsewhere. Loading older articles from Inoreader at the end of the
  list only works with no grouping and newest first.
- Tap the title for the collection drawer; the footer toggles Unread/All and shows sync status
  (tap it to sync now). Swipe or use the page buttons to move between pages of the list.
  Long-press a row for Star, Mark read, Done and Details.
- In an article, tap the middle of the page for the bar: Back, Aa, Star, Done, More. Aa opens
  the text panel (font, size, line spacing, margins, alignment); its settings apply to every
  article. More holds
  Mark read/unread, Tags, Like, Broadcast, Details (including summaries and
  annotations returned by Inoreader) and Load original page.
- Items from Hacker News and Lobsters feeds are link posts: InoInk downloads the linked page and
  extracts its text itself (no credentials, not an Inoreader feature), and caches the thread from
  the site's public JSON. The bar shows an Article | N comments switch (also a link at the end
  of the article); More has Refresh comments. Star, Done and Read apply to the one item.
  Pages that cannot be extracted (paywalls, scripts, PDFs) fall back to the feed text with a note.
- Tap an image to enlarge it; tap again to zoom, swipe to pan. A grey "Image unavailable - retry"
  line is a retryable placeholder.
- After **Done**, an Undo bar sits at the top of the list until you undo it or finish another article.
- Main menu: Sync now, Mark list read (shows a count first), Manage (follow feeds, rename or move feeds
  between folders, rename or delete folders, keep a folder offline, monitoring searches, offline
  storage).
- Slide a finger up or down the left edge of the screen to change the brightness. InoInk starts
  with the brightness and warmth the Kobo was using.
- The Wi-Fi icon in the top bar shows the connection; tap it to switch Wi-Fi on or off. InoInk
  uses the networks the Kobo already knows.
- Quit InoInk before connecting the Kobo to a computer. Suspend stops network work before the
  device sleeps.
