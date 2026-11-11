# InoInk

An unofficial, standalone [Inoreader](https://www.inoreader.com) reader for the **Kobo Libra
Colour** (firmware 4.46.23852), built on the [Plato](https://github.com/baskerville/plato)
document renderer. It reads your Inoreader account directly: no KOReader, no Instapaper or
Wallabag in between, no server of its own.

- Opens on the last collection you used (Starred at first); Unread/All filter, local search,
  collection drawer with folders, feeds and tags.
- Reader: Back, Star, Done and More. **Done marks read and removes the star** (with Undo);
  **Mark read** keeps the star. Opening an article never marks it read.
- Works offline: text and colour images are cached; changes are saved first and sent when
  the network allows. Image enlargement, adjustable text size.
- Two-way sync with Inoreader's documented public API, within its daily quota.

It is **not affiliated with Inoreader or Kobo**, and it does not claim parity with the Inoreader
website: see [doc/INOINK-API.md](doc/INOINK-API.md) for exactly what the public API allows.

## Documentation

| | |
|---|---|
| [doc/INOINK-SETUP.md](doc/INOINK-SETUP.md) | Build, install, sign in, update, remove |
| [doc/INOINK-API.md](doc/INOINK-API.md) | API capability matrix and genuine limitations |
| [doc/INOINK-SYNC.md](doc/INOINK-SYNC.md) | How changes are saved, sent and reconciled |
| [doc/INOINK-ACCEPTANCE.md](doc/INOINK-ACCEPTANCE.md) | What was tested, and how (automated, emulator, device) |
| [doc/UPSTREAM.md](doc/UPSTREAM.md) | Plato provenance and the derivative policy |
| [doc/LICENSES.md](doc/LICENSES.md) | Licenses of everything that ships |
| [doc/PLATO-README.md](doc/PLATO-README.md) | The original Plato README |

## Layout

- `crates/inoink` - renderer-independent library: store, API client, content pipeline, cache, sync.
- `crates/inoink-cli` - computer-side helper: `authorize`, `provision`, `fixtures`, `verify-cache`, `acceptance`.
- `crates/core`, `crates/plato`, `crates/emulator` - inherited renderer, device layer and apps, with the InoInk views in `crates/core/src/view/inoink`.

## License

AGPL-3.0-or-later, as Plato. See `LICENSE-AGPLv3` and `THIRD-PARTY-NOTICES`.
