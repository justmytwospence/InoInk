# Power and Wi-Fi

What InoInk does on a Kobo that costs power: its timers, what it reacts to instead of polling,
when it turns the Wi-Fi on and off, and when it writes to storage. Update this document in the
same change as the code it describes. The structure follows Kobalt's
[power mechanisms](https://github.com/nicoverbruggen/kobalt/blob/main/doc/power-mechanisms.md)
(MIT); no Kobalt code is used.

Code: the app and its timers are in `crates/plato/src/app.rs`, the sync worker in
`crates/inoink/src/runtime.rs`, the sync pass in `crates/inoink/src/sync.rs`.

## Principles

- **Events, not polls.** Sync starts on launch, a wake, the network coming up, a saved change or
  a tap. Nothing checks for new articles on a timer, so InoInk never keeps the Kobo awake to sync.
- **Timers stop with the device.** All timers are threads sleeping on the monotonic clock. While
  the Kobo is suspended they are frozen and wake nothing; the only alarm InoInk sets is the
  automatic power-off (below).
- **The page turn stays local.** Turning a page draws the page and at most saves the position to
  the local database; it never waits for the network or the sync worker.
- **Wi-Fi only when asked for.** InoInk turns the Wi-Fi on for the user, or, with "Turn on Wi-Fi
  to sync", for one sync at a time, and then off again.

## Timers while awake

| Timer | Interval | What it does | Code |
|---|---|---|---|
| Clock | 60 s | Redraws the clock in the list's top bar | `CLOCK_REFRESH_INTERVAL`, `ClockTick` |
| Battery | 299 s | Redraws the battery icon | `BATTERY_REFRESH_INTERVAL`, `BatteryTick` |
| Battery check | 299 s, not while charging | Warns below 10%, powers off below 3% | `TaskId::CheckBattery` |
| Auto-suspend check | 60 s, only when auto-suspend is on (default 30 min) | Puts the Kobo to sleep after that long without input | `AUTO_SUSPEND_REFRESH_INTERVAL`, `MightSuspend` |

Nothing else repeats. The sync worker has no periodic timer: with nothing to do it blocks on its
command queue (`nothing_runs_on_its_own_when_idle`).

## Timers that run for a short time

| Timer | Length | Runs while |
|---|---|---|
| Sleep preparation | 3 s after the sleep screen | The screen settles; then settings are saved, sync stops and the Wi-Fi goes off (`PREPARE_SUSPEND_WAIT_DELAY`) |
| Suspend retry | 15 s, once per wake | After a wake that did not come from the power button, the cover or a cable, InoInk sleeps again (`SUSPEND_WAIT_DELAY`) |
| Automatic power-off | RTC alarm, 3 days of sleep | Set at each sleep, cleared at wake |
| Sync stop at sleep | at most 3 s (5 s for USB) | Sleep waits for the worker to stop its current request and settle the database |
| Change debounce | 2 s after the last change, 10 s after the first | Changes are sent together (`DEBOUNCE`, `MAX_DEBOUNCE`) |
| Network settle | 5 s after the network comes up or the device wakes | A flapping link causes one sync (`NETWORK_SETTLE`) |
| Network watch | every 1 s for at most 90 s after the Wi-Fi is turned on | Reads the address (`scripts/ip.sh`) until there is one, then reports the network up (`watch_network`) |
| Wi-Fi connection deadline | 60 s | A sync that turned the Wi-Fi on waits for a connection (`CONNECT_TIMEOUT`) |
| Local-change connection | 15 min since the last time a sync turned the Wi-Fi on | With "Turn on Wi-Fi to sync", changes made offline turn the Wi-Fi on no more often than this (`LOCAL_CONNECT_INTERVAL`) |
| HTTP | 10 s to connect, 60 s per request (30 s for images); an image is tried 3 times, 0.5 and 1 s apart | A request is in flight |
| SQLite busy wait | at most 5 s | Another connection holds the write lock |
| Reading position | at most every 4 s | After page turns (see below) |
| UI | 150 ms to 4 s, once | Menus closing, holds (0.67 and 1.33 s), notifications (4 s), the end-of-article grace (0.6 s), a stalled download (10 s) |

Sync passes also skip work that is not due (these gate a pass; they are not timers): listings and
the organization at most every 15 minutes for automatic syncs, annotations every 6 hours, reading
progress every hour. See [INOINK-SYNC.md](INOINK-SYNC.md#quota-and-scheduling).

## What InoInk reacts to instead of polling

| Source | How | Code |
|---|---|---|
| Touch, buttons, power button, sleep cover | Input devices, read as they come | `crates/core/src/input.rs` |
| USB and charger | Nickel's `/tmp/nickel-hardware-status` pipe (`usb plug add`, `usb ac add`, ...) | `parse_usb_events` |
| Network up | The same pipe (`network bound`), and the 1 s address watch that runs only after InoInk turns the Wi-Fi on | `parse_usb_events`, `watch_network` |
| Sync results | The worker's update channel; the interface never waits on it | `runtime::Update` |

## What a page turn does not do

- It sends nothing: reading positions and progress made on the Kobo stay on the Kobo, and opening
  or paging an article never marks it read (only the end-of-article popup or a command does).
- It does not wait for the sync worker or the network: the interface reads and writes its own
  database connection while a sync runs (`ui_reads_and_writes_never_wait_for_a_slow_sync`).
- It writes the position at most every 4 seconds (on the next event after a page turn), and when
  the article is left.
- It does not fetch neighbouring articles: the previous and next articles are queued for download
  once, when an article opens.

## Wi-Fi

### The setting

Main menu > Settings > Sync without Wi-Fi:

- **Wait for Wi-Fi** (default). A sync that comes due while the Wi-Fi is off waits. Nothing is
  sent, no request fails, and the waiting sync runs as one pass once the Wi-Fi has an address.
  Tapping sync with the Wi-Fi off says "Turn on Wi-Fi to sync" and syncs once it connects.
- **Turn on Wi-Fi to sync.** A sync that comes due while the Wi-Fi is off turns it on, syncs and
  turns it off again. The status line says "Turning on Wi-Fi" meanwhile.

The worker learns about the network from the app: the network is up when the Wi-Fi has an
address (`Lifecycle::NetworkUp`) and down when InoInk turns the Wi-Fi off (`NetworkDown`). On the
Kobo it starts with the network down, so the launch sync waits for the address.

### When InoInk turns the Wi-Fi on

- At launch, when it was on in the Kobo interface (the launcher passes `INOINK_WIFI`).
- At wake, when the user had it on before sleep.
- When the user taps the Wi-Fi icon.
- With "Turn on Wi-Fi to sync" only, when a sync comes due without a network: at launch, at
  wake, a tap on sync, the fetch after a bulk mark-read, and changes made offline (at most every
  15 minutes; otherwise they wait for the next connection). Nothing else turns it on: opening an
  article that is not downloaded, a link or Load original page needs the Wi-Fi to be on already.

### When InoInk turns the Wi-Fi off

These rules are the contract.

1. **At sleep, always.** The Wi-Fi goes off 3 seconds after the sleep screen, after sync has
   stopped. The user's Wi-Fi comes back at wake; Wi-Fi a sync turned on does not (the wake sync
   asks again if it needs to).
2. **A sync's Wi-Fi goes off when its sync is over.** "Over" means the pass ended (success,
   failure or cancellation) and no other sync is due: changes made during the pass are sent on the
   same connection a few seconds later, then the Wi-Fi goes off. It also goes off when no
   connection comes within 60 seconds, when the setting changes to Wait, at sleep and at quit.
3. **Only Wi-Fi a sync turned on.** InoInk turns the Wi-Fi on for a sync only when it is off, and
   remembers that it did (`Context::wifi_for_sync`). Wi-Fi that was on at launch, or that the
   user turned on, is never turned off by a sync.
4. **The user's tap wins.** Tapping the Wi-Fi icon makes the Wi-Fi the user's: turning it off
   while a sync waits for a connection cancels that request, and the sync does not turn it on
   again until the next sync comes due.

`inoink.log` records `wifi: on for a sync` and `wifi: off after the sync`.

### Failed syncs

After 3 failed syncs in a row, InoInk stops starting syncs on its own (`MAX_FAILURES`). A failed
sync is one that could not reach Inoreader or got an error from it; with "Turn on Wi-Fi to sync",
a connection that did not come within 60 seconds also counts. A paused quota and a sign-in
problem do not count (they have their own state). The status line then says "Sync stopped after 3
failures: tap sync to retry". Changes stay saved and pending. A wake or a tap on sync starts
counting again; a successful sync resets the count.

### MediaTek Kobos

On MediaTek Kobos (Libra Colour, Clara Colour, Clara BW, Elipsa 2E), quitting after InoInk used
the Wi-Fi chip restarts the Kobo instead of the Kobo interface (koreader/koreader#12739; see
[INOINK-SETUP.md](INOINK-SETUP.md#quitting)). With "Turn on Wi-Fi to sync" and a signed-in
account, the launch sync uses the chip, so quitting always restarts the Kobo.

## Storage writes

| What | When |
|---|---|
| Database (`inoink.db`, SQLite WAL, `synchronous=FULL`) | Each change you make, at once, in one transaction. The reading position at most every 4 s after page turns and when leaving an article. Headlines shown on a list page, once per page shown (seen marks). The tab, list and view options when they change. Sync passes as they store what arrives |
| Database checkpoint (`wal_checkpoint(TRUNCATE)`) | At sleep, before USB sharing, at quit |
| Offline content (`cache/`) | During sync passes and when an article is opened online; files are written atomically |
| Settings (`Settings.toml`) | At sleep, before USB sharing and at quit (not on each change) |
| Log (`inoink.log`) | Appended by the launcher and the app: startup, sleep and wake, Wi-Fi and network lines, errors |

While the Kobo sits idle on a list or an article, nothing is written to the database or the
settings.

## Verification

Automated (`crates/inoink/tests/lifecycle.rs`, simulated Inoreader):

- `nothing_runs_on_its_own_when_idle`: no requests without a trigger.
- `flapping_network_events_coalesce_into_one_sync`, `local_changes_are_debounced_into_one_request_batch`.
- `suspend_cancels_work_and_quiesces_before_the_deadline`, `a_hung_network_cannot_hold_up_suspend_beyond_the_deadline`.
- `ui_reads_and_writes_never_wait_for_a_slow_sync`: the page-turn rule.
- `wait_holds_syncs_until_the_network_is_up_and_never_asks_for_wifi`: Wait, including a tapped sync.
- `turn_on_asks_for_wifi_and_hands_it_back_after_the_sync`, `turn_on_keeps_the_wifi_for_changes_made_during_the_sync`, `turn_on_gives_up_when_no_connection_comes`, `turn_on_connects_for_local_changes_at_most_every_interval`, `turning_wifi_off_while_connecting_is_respected`, `switching_to_wait_hands_the_wifi_back`, `switching_to_turn_on_connects_for_a_waiting_sync`: Wi-Fi rules 2 and 4 on the worker's side.
- `three_failures_in_a_row_stop_automatic_syncs_until_a_tap_or_a_wake`, `a_wake_resets_the_failure_count`, `a_success_resets_the_failure_count`, `connections_that_never_come_count_as_failures`.

`crates/core/tests/inoink_views.rs` (`wifi_policy_defaults_to_wait_and_is_saved`): the setting's
default, menu, saving and the status lines.

Not automated: the app's side of the Wi-Fi rules (which Wi-Fi it turns off, rules 1 and 3), the
timers in `app.rs`, and anything about power draw. They are device cases in
[INOINK-ACCEPTANCE.md](INOINK-ACCEPTANCE.md#device-kobo-libra-colour-firmware-44623852), NOT RUN
until checked on a Kobo.
