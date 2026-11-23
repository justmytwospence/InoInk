# Device benchmarks

`inoink-bench` times InoInk on a real Kobo: how long the list takes to appear after launch, list
page turns, opening an article and going back, page turns in an article, a sync with nothing new,
and memory. It runs on the Kobo next to InoInk and sends input the way the hardware does: page
button presses and taps written to the input devices InoInk reads. It watches the framebuffer and
times each action from the end of the input (the finger lifted, the button released: when
InoInk acts) to the first change on the screen and to the last change before the screen stays
still. InoInk draws into the framebuffer before each e-ink update, so these are
the app's times; the e-ink refresh that follows is the panel's and is not included.

The idea comes from [Kobalt's device benchmarks](https://github.com/nicoverbruggen/kobalt) (MIT).
The tool is written for InoInk and needs no change to the app: it uses only the app's existing
`startup:` log lines, the framebuffer, the input devices and `/proc`. Results of device runs go in
[INOINK-ACCEPTANCE.md](INOINK-ACCEPTANCE.md#device-benchmarks), one row per release, never mixed
with emulator results.

## Run it

You need a Kobo with InoInk and NickelMenu installed, connected by USB, and Docker (without
Docker, `install` uses CI's build of HEAD, which must be pushed).

1. Install the benchmark:

   ```sh
   scripts/kobo-bench.sh install
   ```

   It cross-builds the tool with the app's toolchain (`scripts/kobo-bench.sh build` builds it
   alone) and copies it to `.adds/inoink/bench/` with a NickelMenu entry, **InoInk benchmark**.
   Only short file names go to the Kobo, which its disk check leaves alone. InoInk's data,
   settings and releases are not touched.
2. Prepare the Kobo. Portrait, with the Kobo logo at the bottom. Wi-Fi on and InoInk synced
   recently, so the sync finds nothing new. The run opens the first row of the list InoInk starts
   on (the Newsfeed), so that article should have several pages, and the list more than one page.
   The run opens, pages through and closes articles and syncs; it does not mark, star or archive
   anything. Then quit InoInk.
3. Eject the Kobo, choose **InoInk benchmark** in NickelMenu and do not touch the screen or the
   buttons until the run ends. It starts InoInk, waits for its list and for the startup sync to
   finish (the screen still for 8 seconds), then runs, in order:

   | Scenario | What it does | Measures |
   | --- | --- | --- |
   | `startup.*` | Launches InoInk | The app's own `startup:` log lines: database open, list built, `first-list-frame-sent` (launch to list) |
   | `memory.list` | | Resident and peak memory with the list shown |
   | `sync.idle` | Taps the sync button 3 times | From the tap to the syncing icon (`first change`) and back to the plain icon (`until done`); bytes over Wi-Fi |
   | `list.page` | The page buttons, forward and back, 5 times each | First change and settled, per press |
   | `article.open` | Taps the first row, 5 times | First change and settled |
   | `page.turn` | In the first article: the page buttons, forward and back, 10 presses | First change and settled, per press |
   | `memory.article` | | Memory after the page turns |
   | `article.back` | Taps the middle (shows the bars), then the title, which goes back | First change and settled for the title tap: back to the list |
   | `memory.end` | | Memory at the end |

   A run takes a few minutes. It leaves InoInk on the list; quit it as usual. A scenario that
   cannot run is recorded as skipped with the reason (Wi-Fi off; the list has one page).
4. Connect the Kobo and collect the results:

   ```sh
   scripts/kobo-bench.sh collect
   ```

   It copies the new result files to `target/kobo-bench/results/`, prints a Markdown table for
   each, and the number of Inoreader requests in the last sync as InoInk recorded it (also in
   Settings > Inoreader API budget). `scripts/kobo-bench.sh report FILE...` prints the table
   again; several files of the same release, device and firmware pool into one table (median and
   range over all samples).
5. Copy the table into [INOINK-ACCEPTANCE.md](INOINK-ACCEPTANCE.md#device-benchmarks) under the
   release it measured. `scripts/kobo-bench.sh remove` takes the benchmark and its menu entry off
   the Kobo once its results are collected.

Run it at least twice per release and note the spread: e-ink devices vary with temperature,
battery and background work. Compare a release with an earlier release on the same device and
firmware only.

## How it times

The tool keeps a signature of the screen: four pixels sampled in each 24-pixel cell (4-pixel cells
around the sync icon). Before each action it takes a signature, sends the input, then polls every
few milliseconds. **First change** is the first poll that differs; **settled** is the last change
before 800 ms without one. Reading the framebuffer costs a few milliseconds per poll, so times are
good to about 10 ms. The clock and the battery in the top bar are left out (`ignore`).

Each action is checked before the run goes on, so a wrong position never sends a string of taps
into the wrong screen: opening an article must change at least half the screen, a page turn at
least 15%, showing the bars less than half, and the sync tap must change the sync icon first and
nothing below the top bar. Otherwise the run stops at once, writes why, and leaves InoInk where it
is. A stopped run is reported as such by `collect`. One exception: when the sync tap changes
nothing, a sync may still be running, so the run waits for it to end and taps once more.

## Settings

`.adds/inoink/bench/bench.cfg` lists every setting, commented out, with its default for the Libra
Colour in portrait; `install` never replaces an edited one. The defaults follow InoInk's layout
(the top bar's slots, the first row below the tabs), computed from the screen size, so they also
fit other screen sizes. After a layout change, check them against a screenshot of the preview
(`scripts/preview-macos.sh --capture`, then `01-newsfeed.png`: positions are in its pixels).
`inoink-bench defaults PRODUCT` prints the defaults for another model, for example `spaBW`.

## Devices

Taps need the touch panel's layout. The tool knows it for the Libra Colour (`monza`), the Clara BW
and Colour, Clara 2E, Clara HD, Libra 2, Libra H2O, Forma and Elipsa 2E, from InoInk's device
table, and refuses other models. The Sage and the Elipsa draw outside `/dev/fb0` and are not
supported. Only the Libra Colour is the owner's device; on the others the tool is untested.

## Results

Each run writes `.adds/inoink/bench/results/<time>-<release>.jsonl`, one JSON object per line,
written as it goes:

- `header`: release, model, firmware, start time and the main settings;
- `sample`: a scenario, its sample number and its measures (`first_ms`, `settled_ms`, `total_ms`,
  `ms`, `rx_bytes`, `tx_bytes`, `rss_kib`, `hwm_kib`) and `changed`, the share of the screen that
  changed;
- `skip`: a scenario that did not run, with the reason;
- `error`: why the run stopped;
- `end`: whether the run finished, and how long it took.

`results/bench.log` keeps the tool's own messages.

## Checks

The tool's tests run on the computer (`scripts/test-inoink.sh`, CI): the touch-coordinate
transform against InoInk's device table for every supported model and rotation, the input event
layout, signatures and differences, the settings and their defaults (the shipped `bench.cfg`
must match them), reading the startup log, the report, and whole runs against a simulated InoInk
on a simulated clock, including runs that must stop. `tests/kobo-bench.sh` checks what
`kobo-bench.sh` copies to and removes from a Kobo. None of this is evidence about the hardware.
