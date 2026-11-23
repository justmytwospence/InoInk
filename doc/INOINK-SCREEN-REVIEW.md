# Screen review

Screen review is a local web page for reviewing UI changes one screen at a time. It shows the
screenshots of the scripted emulator tour (or any folder of PNGs), lets the reviewer pin notes to
places on a screen, and gives the notes back on the command line for the next round. Before and
after can be compared on the same screen. It needs Node.js and nothing else: no packages, no
network. The idea comes from [Kobalt's screen review](https://github.com/nicoverbruggen/kobalt)
(MIT); the tool is written for InoInk.

Everything is under `scripts/review/`: `review.mjs` (the command line and a server for this
computer only), `page.html` (the page) and `review.test.mjs` (its checks). Review data lives in
`target/screen-review/` (set `INOINK_REVIEW_DIR` for another folder).

## Workflow

Run these commands from the repository root.

1. Capture the screens. On an Apple Silicon Mac:

   ```sh
   scripts/preview-macos.sh --capture                    # sample articles, Libra Colour
   scripts/preview-macos.sh --capture --device spaBW:391 # the same tour as a Clara BW
   scripts/preview-macos.sh --capture --inoreader        # your snapshot of real articles
   ```

   `--capture` runs the scripted tour without a window on a throwaway copy of the data with
   default settings, so every capture starts from the same state and the preview's own data,
   settings and screenshots are left alone. The screens go to
   `target/macos-preview/captures/<sample|inoreader>/<codename>/`, replacing the earlier capture of
   that device. The tour is written for the sample articles: with `--inoreader`, the steps that
   open a sample article by its id show something else. The snapshot holds your real articles, so
   its screenshots stay on this computer. On Linux, or with Docker,
   `scripts/build-inoink.sh --run-emulator --data target/inoink-fixtures` and
   `scripts/emulator-matrix.sh` write the same tour to `target/inoink-emulator-shots/<codename>/`.
2. Create a round from the captures:

   ```sh
   node scripts/review/review.mjs round
   ```

   Without `--source`, it takes `target/macos-preview/captures/sample`, or
   `target/inoink-emulator-shots` when that does not exist. Give `--source DIR` (repeatable) for other
   folders. A folder holds one subfolder of PNGs per device, named by its codename, or PNGs
   directly; then `--device NAME` names the device (default: the folder's name). Screenshots saved
   with **S** in the interactive preview are such a folder:
   `--source target/macos-preview/state/screenshots --device monza`.
3. Start the server and open the page:

   ```sh
   node scripts/review/review.mjs serve
   ```

   The page is at `http://localhost:8765/` (`--port` changes the port). The server listens on this
   computer only and answers only requests addressed to `localhost` or `127.0.0.1`.
4. When every screen is seen, the reviewer presses **Finish review**. Read the feedback:

   ```sh
   node scripts/review/review.mjs notes
   ```

   It lists each screen that needs changes, is queued, or has a reply waiting: the path of its
   image and of its Before image, the note and each pin with its position in the image's pixels.
   Its second line says whether the reviewer finished the round. A change after Finish review
   reopens the round.
5. Fix the screens, capture again and create the next round. A note that needs no change, such as
   a question, gets a reply instead:

   ```sh
   node scripts/review/review.mjs reply monza/09b-article-end "Yes: Archive is remembered."
   ```

   The reply waits in `target/screen-review/replies.json` and is shown with the next round. The
   open page notices a new round within 5 seconds and offers it; the server needs no restart
   (only after a change to the review tool itself).

## Queue screens for the reviewer

```sh
node scripts/review/review.mjs ask 09b-article-end "Is the Archive toggle clear enough?"
node scripts/review/review.mjs ask --clear
```

The screen is a name, which queues it on every device, or a full id such as
`monza/09b-article-end`. The question shows above the notes, and the status filter **Queued for
you** lists the queue; the first time a round with a queue opens, the page selects that filter.
The queue belongs to the latest round; a new round starts without one.

## Rounds

Each `round` creates the next round in `target/screen-review/rounds/NNN/`. Only the latest round
takes feedback; earlier rounds open read only, and a page left open on a round that is superseded
turns read only with a banner offering the new one.

A new round compares each image with the previous round:

- An unchanged image keeps the reviewer's verdict, note and pins; the page says which round they
  came from. Changing them makes them this round's own.
- A changed image starts unseen and without notes. The earlier verdict and notes are shown in grey
  below the notes, and **Compare with: Previous round** shows the earlier image with its pins.
- A screen with a reply starts unseen and without notes, and shows the earlier notes with the
  reply under **Answered**. The answer is carried to later rounds until the reviewer has seen it.
- A screen not in the previous round is new.

A capture of only some screens keeps the others, with their feedback, from the previous round
(marked kept). Add `--fresh` for a round with only the captured screens. Images are stored once
in `target/screen-review/images/`, named by their SHA-256, so rounds share unchanged images.
Delete `target/screen-review/` to start again from round 1.

## Before and after

Capture the screens before the change, then record them as the Before set:

```sh
node scripts/review/review.mjs baseline --source DIR --commit "$(git rev-parse origin/main)"
```

A simple way to get DIR: capture in the main checkout (or another worktree on `main`) and point
`--source` at its `target/macos-preview/captures/sample`. A screen the change adds or renames
has no Before image under its own name; map it with `--match NEW=OLD` (screen names or full ids),
for example `--match 09c-up-next=09b-article-end`. The page then offers **Compare with: Before**
for every screen with a Before image, with the commit in the caption.

## The page

The page shows one screen at a time: the screen list on the left with each screen's state, the
image in the middle and the notes on the right. Click the latest image to pin a note to that
place; each pin has its own note box and a Remove button, and the note box at the bottom is for
the screen as a whole. A note or a pin marks the screen **Needs changes**; a screen seen without
either is approved. Moving to another screen marks the current one seen. Everything saves at
once (typing after a short pause).

Under the image, **Compare with** chooses None, Before or the previous round. **Hold to compare**
(or holding **C**) shows the comparison in place of the latest image, **Side by side** (**S**)
shows both at one scale, and **Differences** (**D**) tints in red the pixels that differ (same
size only). **Fit** and **100%** (**Z**) switch the zoom. The filters show all screens, unseen,
seen, needs changes, new or changed, queued, or the screens that differ from the comparison; with
several devices, a second menu picks one.

| Key | Action |
| --- | --- |
| Left, Right | Previous and next screen (marks the current one seen) |
| Space | Mark seen and go to the next screen |
| R | Write the note for this screen |
| C (hold) | Show the comparison instead of the latest image |
| S | Comparison side by side |
| D | Tint what differs from the comparison |
| Z | Fit or 100% |
| Esc | Leave the note box |
| ? | All shortcuts |

## Files

| Path | Content |
| --- | --- |
| `target/screen-review/rounds/NNN/round.json` | The round's screens, their images and status, and what the previous round said |
| `target/screen-review/rounds/NNN/feedback.json` | Seen flags, notes and pins by screen (pins as fractions of the image), and when the round was finished |
| `target/screen-review/rounds/NNN/queue.json` | Screens queued for the reviewer, with their questions |
| `target/screen-review/baseline.json` | The Before set: images by screen, the commit, name mappings |
| `target/screen-review/replies.json` | Replies waiting for the next round |
| `target/screen-review/images/` | The images of all rounds |

A screen is identified by device and name, for example `monza/06-article`.

## Checks

`node --test scripts/review/review.test.mjs` (part of `scripts/test-inoink.sh` and CI) covers
rounds and carry-over, replies, the queue, Before mappings, the notes output and the server (only
the latest round takes feedback, writes need JSON from the page's own origin, other host names are
refused). Each check works in its own temporary folder.
