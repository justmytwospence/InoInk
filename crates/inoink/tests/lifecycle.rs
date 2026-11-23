mod common;
use common::fake::*;
use common::*;
use inoink::api::*;
use inoink::cache::{Cache, CacheConfig};
use inoink::content::images::{Fetched, Fetcher};
use inoink::error::Result;
use inoink::model::*;
use inoink::runtime::*;
use inoink::store::Store;
use inoink::sync::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};
use url::Url;

const NOW: i64 = 1_800_000_000;
const FEED: &str = "feed/https://blog.example/rss";

struct NoImages;
impl Fetcher for NoImages {
    fn get(&self, _u: &Url, _m: usize) -> Result<Fetched> {
        Ok(Fetched { status: 404, ..Default::default() })
    }
}

/// Delegates to the fake server after a delay; counts requests; can be told to hang or to
/// fail as if there were no network.
struct Slow {
    inner: Arc<Fake>,
    delay: Duration,
    requests: AtomicUsize,
    hang: AtomicBool,
    fail: AtomicBool,
}
impl Transport for Slow {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            return Err(inoink::error::Error::Offline);
        }
        let ms = if self.hang.load(Ordering::SeqCst) { 1500 } else { self.delay.as_millis() as u64 };
        std::thread::sleep(Duration::from_millis(ms));
        self.inner.send(req)
    }
}

struct Env {
    dir: tempfile::TempDir,
    fake: Arc<Fake>,
    slow: Arc<Slow>,
    clock: Arc<FixedClock>,
    handle: Option<Handle>,
    updates: Receiver<Update>,
}

fn engine_for(dir: &Path, slow: &Arc<Slow>, clock: &Arc<FixedClock>, tokens: &Arc<MemTokens>) -> SyncEngine {
    let mut acfg = AppConfig::new("cid", "secret", "http://localhost/cb");
    acfg.base_url = "https://example.test/reader/api/0".into();
    acfg.token_url = "https://example.test/oauth2/token".into();
    let api = ApiClient::new(acfg, slow.clone(), clock.clone(), tokens.clone()).unwrap();
    let store = Store::open(&dir.join("db.sqlite"), "42").unwrap();
    let cache = Cache::new(&dir.join("cache"), "42", CacheConfig::default());
    SyncEngine::new(store, api, cache, Arc::new(NoImages), clock.clone(), SyncConfig { min_interval_secs: 0, ..Default::default() })
}

fn env(delay_ms: u64, articles: u64) -> Env {
    env_with(delay_ms, articles, Options::default())
}

fn env_with(delay_ms: u64, articles: u64, opts: Options) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    fake.with(|s| s.subs.push(SSub { id: FEED.into(), title: "Blog".into(), url: String::new(), folders: vec![], first_usec: 0 }));
    for i in 1..=articles {
        fake.art(i, (NOW - i as i64 * 60) * 1_000_000, FEED);
    }
    let slow = Arc::new(Slow {
        inner: fake.clone(),
        delay: Duration::from_millis(delay_ms),
        requests: AtomicUsize::new(0),
        hang: AtomicBool::new(false),
        fail: AtomicBool::new(false),
    });
    let clock = FixedClock::new(NOW);
    let tokens = Arc::new(MemTokens::default());
    *tokens.tokens.lock().unwrap() =
        Some(Tokens { access_token: "acc".into(), refresh_token: "ref".into(), expires_at: NOW + 3600, scope: String::new() });
    let engine = engine_for(dir.path(), &slow, &clock, &tokens);
    let (utx, urx) = channel();
    let path: PathBuf = dir.path().join("db.sqlite");
    let handle = start_with(engine, "42", Box::new(move || Store::open(&path, "42")), utx, opts);
    Env { dir, fake, slow, clock, handle: Some(handle), updates: urx }
}

impl Env {
    fn h(&self) -> &Handle {
        self.handle.as_ref().unwrap()
    }
    fn ui_store(&self) -> Store {
        Store::open(&self.dir.path().join("db.sqlite"), "42").unwrap()
    }
    fn wait_cycles(&self, n: usize, timeout: Duration) -> Vec<CycleReport> {
        let end = Instant::now() + timeout;
        let mut out = vec![];
        while out.len() < n {
            let left = end.saturating_duration_since(Instant::now());
            match self.updates.recv_timeout(left) {
                Ok(Update::Cycle(c)) => out.push(c),
                Ok(_) => {}
                Err(_) => break,
            }
        }
        out
    }
    fn drain(&self) -> Vec<Update> {
        self.updates.try_iter().collect()
    }
    /// Collects updates until `pred` matches one (returned last) or `timeout` passes.
    fn until(&self, timeout: Duration, pred: impl Fn(&Update) -> bool) -> Vec<Update> {
        let end = Instant::now() + timeout;
        let mut out = vec![];
        while let Ok(u) = self.updates.recv_timeout(end.saturating_duration_since(Instant::now())) {
            let hit = pred(&u);
            out.push(u);
            if hit {
                break;
            }
        }
        out
    }
    fn requests(&self) -> usize {
        self.slow.requests.load(Ordering::SeqCst)
    }
}

/// Starts without a network, as on the Kobo, with short delays for the tests.
fn offline(wifi: WifiPolicy) -> Options {
    Options {
        wifi,
        online: false,
        settle: Duration::from_millis(50),
        connect_timeout: Duration::from_secs(2),
        local_connect_interval: Duration::from_secs(4),
    }
}

fn wifi_requests(us: &[Update]) -> Vec<bool> {
    us.iter().filter_map(|u| if let Update::Wifi(on) = u { Some(*on) } else { None }).collect()
}

#[test]
fn ui_reads_and_writes_never_wait_for_a_slow_sync() {
    let e = env(150, 20);
    e.h().submit(Command::SyncNow);
    // Wait until the sync is clearly in progress.
    let t0 = Instant::now();
    while e.slow.requests.load(Ordering::SeqCst) < 2 && t0.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(e.slow.requests.load(Ordering::SeqCst) >= 2);
    let mut ui = e.ui_store();
    let mut worst = Duration::ZERO;
    let mut turns = 0;
    let id = ArticleId(1);
    let end = Instant::now() + Duration::from_millis(600);
    while Instant::now() < end {
        let t = Instant::now();
        // A "page turn": read the list and persist the reading position.
        let _ = ui.query_articles(&ArticleQuery { limit: 20, ..Default::default() }).unwrap();
        if ui.has_article(id).unwrap() {
            ui.save_position(
                id,
                &ReadingPosition { content_hash: "h".into(), offset: turns as u64, anchor: String::new(), font_size: None },
            )
            .unwrap();
        }
        worst = worst.max(t.elapsed());
        turns += 1;
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(turns > 20, "{turns} turns");
    assert!(worst < Duration::from_millis(100), "slowest UI operation took {worst:?} while the sync was running");
    assert!(e.wait_cycles(1, Duration::from_secs(60)).len() == 1);
    e.h().set_lifecycle(Lifecycle::Suspend);
}

#[test]
fn suspend_cancels_work_and_quiesces_before_the_deadline() {
    let e = env(100, 30);
    e.h().submit(Command::SyncNow);
    std::thread::sleep(Duration::from_millis(250));
    let t = Instant::now();
    e.h().set_lifecycle(Lifecycle::Suspend);
    assert!(e.h().quiesce(Duration::from_secs(3)), "worker went idle");
    let waited = t.elapsed();
    assert!(waited < Duration::from_millis(1500), "quiesced in {waited:?}");
    let after = e.slow.requests.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(e.slow.requests.load(Ordering::SeqCst), after, "no requests after suspend");
    // Work submitted while suspended waits; local changes stay durable.
    let mut ui = e.ui_store();
    ui.apply_intent(ArticleId(1), &Membership::Starred, true).unwrap();
    e.h().changed();
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!(e.slow.requests.load(Ordering::SeqCst), after, "a suspended device does not sync");
}

#[test]
fn a_hung_network_cannot_hold_up_suspend_beyond_the_deadline() {
    let e = env(10, 5);
    e.slow.hang.store(true, Ordering::SeqCst);
    e.h().submit(Command::SyncNow);
    std::thread::sleep(Duration::from_millis(200));
    e.h().set_lifecycle(Lifecycle::Suspend);
    let t = Instant::now();
    let ok = e.h().quiesce(Duration::from_millis(300));
    assert!(!ok, "the in-flight request has not returned yet");
    assert!(t.elapsed() < Duration::from_millis(700), "the caller was released at its deadline");
    e.slow.hang.store(false, Ordering::SeqCst);
}

#[test]
fn nothing_runs_on_its_own_when_idle() {
    let e = env(5, 3);
    assert_eq!(e.wait_cycles(1, Duration::from_secs(10)).len(), 1, "launch sync");
    let n = e.slow.requests.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(e.slow.requests.load(Ordering::SeqCst), n, "no timers, no polling: the device may sleep");
}

#[test]
fn flapping_network_events_coalesce_into_one_sync() {
    let e = env(1, 3);
    assert_eq!(e.wait_cycles(1, Duration::from_secs(10)).len(), 1);
    e.drain();
    for _ in 0..10 {
        e.h().set_lifecycle(Lifecycle::NetworkUp);
        e.h().set_lifecycle(Lifecycle::NetworkDown);
    }
    e.h().set_lifecycle(Lifecycle::NetworkUp);
    let cycles = e.wait_cycles(2, Duration::from_secs(NETWORK_SETTLE.as_secs() + 3));
    assert_eq!(cycles.len(), 1, "one sync after the link settled, not eleven");
}

#[test]
fn local_changes_are_debounced_into_one_request_batch() {
    let e = env(1, 5);
    assert_eq!(e.wait_cycles(1, Duration::from_secs(10)).len(), 1);
    e.fake.clear_log();
    let mut ui = e.ui_store();
    for i in 1..=5u64 {
        ui.apply_intent(ArticleId(i), &Membership::Read, true).unwrap();
        e.h().changed();
        std::thread::sleep(Duration::from_millis(100));
    }
    let cycles = e.wait_cycles(1, Duration::from_secs(8));
    assert_eq!(cycles.len(), 1);
    assert_eq!(cycles[0].sent_batches, 1);
    assert_eq!(e.fake.count("edit-tag"), 1, "five changes, one request");
    assert!((1..=5u64).all(|i| e.fake.a(i).read));
}

#[test]
fn a_refill_sends_changes_and_fetches_at_once() {
    let e = env(1, 5);
    assert_eq!(e.wait_cycles(1, Duration::from_secs(10)).len(), 1);
    e.fake.clear_log();
    let mut ui = e.ui_store();
    ui.apply_intent(ArticleId(1), &Membership::Read, true).unwrap();
    e.h().submit(Command::Refill);
    let cycles = e.wait_cycles(1, Duration::from_secs(5));
    assert_eq!(cycles.len(), 1);
    assert_eq!(e.fake.count("edit-tag"), 1, "the change is sent");
    assert!(e.fake.count("/stream/items/ids") >= 1, "and the next articles are fetched");
}

#[test]
fn usb_sharing_closes_the_database_completely_and_reopens_afterwards() {
    let mut e = env(2, 4);
    assert_eq!(e.wait_cycles(1, Duration::from_secs(10)).len(), 1);
    {
        let mut ui = e.ui_store();
        ui.apply_intent(ArticleId(1), &Membership::Starred, true).unwrap();
        // The UI closes its own handle before sharing (end of scope).
        let _ = &mut ui;
    }
    e.h().set_lifecycle(Lifecycle::UsbShare);
    assert!(e.h().quiesce(Duration::from_secs(5)));
    std::thread::sleep(Duration::from_millis(100));
    // What the host sees: the database file alone, with no -wal/-shm needed, holding everything.
    let db = e.dir.path().join("db.sqlite");
    let copy_dir = tempfile::tempdir().unwrap();
    let copy = copy_dir.path().join("copy.sqlite");
    std::fs::copy(&db, &copy).unwrap();
    {
        let s = Store::open(&copy, "42").unwrap();
        assert!(s.has_article(ArticleId(1)).unwrap());
        assert_eq!(s.pending_for(ArticleId(1)).unwrap(), vec![(Membership::Starred, true)]);
    }
    let wal = e.dir.path().join("db.sqlite-wal");
    assert!(!wal.exists() || std::fs::metadata(&wal).unwrap().len() == 0, "WAL folded into the main file");
    // While shared, nothing syncs, even on request.
    let n = e.slow.requests.load(Ordering::SeqCst);
    e.h().submit(Command::SyncNow);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(e.slow.requests.load(Ordering::SeqCst), n);
    // After unsharing it reopens, recovers and sends the pending change.
    e.h().set_lifecycle(Lifecycle::UsbUnshare);
    e.h().submit(Command::SyncNow);
    let cycles = e.wait_cycles(1, Duration::from_secs(10));
    assert_eq!(cycles.len(), 1);
    assert!(e.fake.a(1).starred);
    e.handle.take().unwrap().shutdown();
}

#[test]
fn pending_work_survives_shutdown_and_is_sent_by_the_next_run() {
    let mut e = env(1, 3);
    assert_eq!(e.wait_cycles(1, Duration::from_secs(10)).len(), 1);
    {
        let mut ui = e.ui_store();
        ui.apply_intent(ArticleId(2), &Membership::Read, true).unwrap();
    }
    e.h().changed();
    e.handle.take().unwrap().shutdown(); // before the debounce fires
    assert!(!e.fake.a(2).read);
    // A new process starts with the same database.
    let tokens = Arc::new(MemTokens::default());
    *tokens.tokens.lock().unwrap() =
        Some(Tokens { access_token: "acc".into(), refresh_token: "ref".into(), expires_at: NOW + 3600, scope: String::new() });
    let engine = engine_for(e.dir.path(), &e.slow, &e.clock, &tokens);
    let (utx, urx) = channel();
    let path = e.dir.path().join("db.sqlite");
    let h = start(engine, "42", Box::new(move || Store::open(&path, "42")), utx);
    let t = Instant::now();
    while !matches!(urx.recv_timeout(Duration::from_secs(10)), Ok(Update::Cycle(_))) {
        assert!(t.elapsed() < Duration::from_secs(10));
    }
    assert!(e.fake.a(2).read);
    h.shutdown();
}

#[test]
fn browse_and_open_online_run_on_the_worker_and_report_back() {
    let e = env(1, 6);
    assert_eq!(e.wait_cycles(1, Duration::from_secs(10)).len(), 1);
    e.fake.with(|s| s.page_cap = 4);
    e.h().submit(Command::Browse { collection: Collection::All, continuation: None });
    let mut got = None;
    for _ in 0..50 {
        match e.updates.recv_timeout(Duration::from_secs(5)) {
            Ok(Update::Browsed { next, .. }) => {
                got = Some(next);
                break;
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    assert!(got.is_some());
    e.h().submit(Command::CacheNow(ArticleId(3)));
    loop {
        match e.updates.recv_timeout(Duration::from_secs(5)).unwrap() {
            Update::Cached(id, st) => {
                assert_eq!(id, ArticleId(3));
                assert!(st.readable_offline());
                break;
            }
            Update::CacheFailed(_, m) => panic!("{m}"),
            _ => {}
        }
    }
}

#[test]
fn wait_holds_syncs_until_the_network_is_up_and_never_asks_for_wifi() {
    let e = env_with(1, 3, offline(WifiPolicy::Wait));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(e.requests(), 0, "the launch sync waits for the network");
    let mut ui = e.ui_store();
    ui.apply_intent(ArticleId(1), &Membership::Read, true).unwrap();
    e.h().changed();
    e.h().submit(Command::SyncNow);
    let us = e.until(Duration::from_secs(3), |u| matches!(u, Update::WaitingForWifi));
    assert!(us.iter().any(|u| matches!(u, Update::WaitingForWifi)), "a tapped sync says it waits: {us:?}");
    assert_eq!(e.requests(), 0);
    e.h().set_lifecycle(Lifecycle::NetworkUp);
    let cycles = e.wait_cycles(2, Duration::from_secs(3));
    assert_eq!(cycles.len(), 1, "everything that waited goes in one pass");
    assert!(cycles[0].error.is_none());
    assert!(!cycles[0].skipped_interval, "the waiting sync the user asked for refreshes");
    assert!(e.fake.a(1).read);
    assert!(wifi_requests(&e.drain()).is_empty());
}

#[test]
fn turn_on_asks_for_wifi_and_hands_it_back_after_the_sync() {
    let e = env_with(1, 3, offline(WifiPolicy::TurnOn));
    let us = e.until(Duration::from_secs(2), |u| matches!(u, Update::Wifi(true)));
    assert_eq!(wifi_requests(&us), vec![true], "the launch sync asks for the Wi-Fi");
    let connecting = |u: &Update| *u == Update::Status(SyncStatus::Connecting);
    assert!(us.iter().any(connecting) || e.until(Duration::from_millis(200), connecting).iter().any(connecting));
    assert_eq!(e.requests(), 0);
    e.h().set_lifecycle(Lifecycle::NetworkUp);
    let us = e.until(Duration::from_secs(3), |u| matches!(u, Update::Wifi(false)));
    let cycle = us.iter().position(|u| matches!(u, Update::Cycle(c) if c.error.is_none()));
    let release = us.iter().position(|u| matches!(u, Update::Wifi(false)));
    assert!(cycle.is_some() && release.is_some() && cycle < release, "synced, then handed back: {us:?}");
    // The app turns it off.
    e.h().set_lifecycle(Lifecycle::NetworkDown);
    std::thread::sleep(Duration::from_millis(300));
    assert!(wifi_requests(&e.drain()).is_empty(), "nothing asks again on its own");
}

#[test]
fn turn_on_keeps_the_wifi_for_changes_made_during_the_sync() {
    let e = env_with(150, 3, offline(WifiPolicy::TurnOn));
    e.until(Duration::from_secs(2), |u| matches!(u, Update::Wifi(true)));
    e.h().set_lifecycle(Lifecycle::NetworkUp);
    e.until(Duration::from_secs(2), |u| *u == Update::Status(SyncStatus::Syncing));
    let mut ui = e.ui_store();
    // The worker may be writing at this moment; the UI would try again on its next action.
    let mut tries = 0;
    while ui.apply_intent(ArticleId(2), &Membership::Read, true).is_err() {
        tries += 1;
        assert!(tries < 40, "the database stayed locked");
        std::thread::sleep(Duration::from_millis(50));
    }
    e.h().changed();
    let us = e.until(Duration::from_secs(6), |u| matches!(u, Update::Wifi(false)));
    let cycles = us.iter().filter(|u| matches!(u, Update::Cycle(_))).count();
    assert_eq!(cycles, 2, "the change went out before the Wi-Fi was handed back: {us:?}");
    assert!(e.fake.a(2).read);
}

#[test]
fn turn_on_gives_up_when_no_connection_comes() {
    let e = env_with(1, 3, Options { connect_timeout: Duration::from_millis(400), ..offline(WifiPolicy::TurnOn) });
    let us = e.until(Duration::from_secs(3), |u| matches!(u, Update::Wifi(false)));
    assert_eq!(wifi_requests(&us), vec![true, false], "asked, then handed back at the deadline");
    assert!(e
        .until(Duration::from_millis(300), |u| *u == Update::Status(SyncStatus::Offline))
        .contains(&Update::Status(SyncStatus::Offline)));
    assert_eq!(e.requests(), 0);
}

#[test]
fn turn_on_connects_for_local_changes_at_most_every_interval() {
    let e = env_with(1, 3, offline(WifiPolicy::TurnOn));
    e.until(Duration::from_secs(2), |u| matches!(u, Update::Wifi(true)));
    let connected = Instant::now();
    e.h().set_lifecycle(Lifecycle::NetworkUp);
    e.until(Duration::from_secs(3), |u| matches!(u, Update::Wifi(false)));
    e.h().set_lifecycle(Lifecycle::NetworkDown);
    let mut ui = e.ui_store();
    ui.apply_intent(ArticleId(3), &Membership::Read, true).unwrap();
    e.h().changed();
    // The debounce (2 s) ends before the interval (4 s): the change waits for the interval.
    let us = e.until(Duration::from_secs(6), |u| matches!(u, Update::Wifi(true)));
    assert_eq!(wifi_requests(&us), vec![true]);
    let after = connected.elapsed();
    // Not at the end of the debounce (about 2.3 s), but at the interval.
    assert!(after >= Duration::from_secs(3), "asked again after {after:?}");
    e.h().set_lifecycle(Lifecycle::NetworkUp);
    e.until(Duration::from_secs(3), |u| matches!(u, Update::Wifi(false)));
    assert!(e.fake.a(3).read);
}

#[test]
fn turning_wifi_off_while_connecting_is_respected() {
    let e = env_with(1, 3, Options { connect_timeout: Duration::from_secs(30), ..offline(WifiPolicy::TurnOn) });
    e.until(Duration::from_secs(2), |u| matches!(u, Update::Wifi(true)));
    e.h().set_lifecycle(Lifecycle::NetworkDown);
    std::thread::sleep(Duration::from_millis(300));
    assert!(wifi_requests(&e.drain()).is_empty(), "no second request after the user switched the Wi-Fi off");
    assert_eq!(e.requests(), 0);
}

#[test]
fn switching_to_wait_hands_the_wifi_back() {
    let e = env_with(1, 3, Options { connect_timeout: Duration::from_secs(30), ..offline(WifiPolicy::TurnOn) });
    e.until(Duration::from_secs(2), |u| matches!(u, Update::Wifi(true)));
    e.h().submit(Command::SetWifiPolicy(WifiPolicy::Wait));
    let us = e.until(Duration::from_secs(2), |u| matches!(u, Update::Wifi(false)));
    assert_eq!(wifi_requests(&us), vec![false]);
    // The launch sync still waits, and goes when the Wi-Fi comes on.
    e.h().set_lifecycle(Lifecycle::NetworkUp);
    assert_eq!(e.wait_cycles(1, Duration::from_secs(3)).len(), 1);
}

#[test]
fn switching_to_turn_on_connects_for_a_waiting_sync() {
    let e = env_with(1, 3, offline(WifiPolicy::Wait));
    std::thread::sleep(Duration::from_millis(200));
    e.h().submit(Command::SetWifiPolicy(WifiPolicy::TurnOn));
    let us = e.until(Duration::from_secs(2), |u| matches!(u, Update::Wifi(true)));
    assert_eq!(wifi_requests(&us), vec![true]);
}

#[test]
fn three_failures_in_a_row_stop_automatic_syncs_until_a_tap_or_a_wake() {
    let e = env_with(1, 3, Options { settle: Duration::from_millis(50), ..Options::default() });
    e.slow.fail.store(true, Ordering::SeqCst);
    assert_eq!(e.wait_cycles(1, Duration::from_secs(5)).len(), 1, "launch fails");
    e.h().set_lifecycle(Lifecycle::NetworkUp);
    assert_eq!(e.wait_cycles(1, Duration::from_secs(3)).len(), 1);
    e.h().set_lifecycle(Lifecycle::NetworkUp);
    let us = e.until(Duration::from_secs(3), |u| matches!(u, Update::Cycle(_)));
    assert!(us.contains(&Update::Status(SyncStatus::Stopped)), "the third failure stops: {us:?}");
    let n = e.requests();
    e.h().set_lifecycle(Lifecycle::NetworkUp);
    let mut ui = e.ui_store();
    ui.apply_intent(ArticleId(1), &Membership::Read, true).unwrap();
    e.h().changed();
    assert!(e.wait_cycles(1, Duration::from_millis(2600)).is_empty(), "no retries on their own");
    assert_eq!(e.requests(), n);
    // A tap tries again.
    e.slow.fail.store(false, Ordering::SeqCst);
    e.h().submit(Command::SyncNow);
    let c = e.wait_cycles(1, Duration::from_secs(3));
    assert!(c.len() == 1 && c[0].error.is_none());
    assert!(e.fake.a(1).read);
}

#[test]
fn a_wake_resets_the_failure_count() {
    let e = env_with(1, 3, Options { settle: Duration::from_millis(50), ..Options::default() });
    e.slow.fail.store(true, Ordering::SeqCst);
    assert_eq!(e.wait_cycles(1, Duration::from_secs(5)).len(), 1);
    for _ in 0..2 {
        e.h().set_lifecycle(Lifecycle::NetworkUp);
        assert_eq!(e.wait_cycles(1, Duration::from_secs(3)).len(), 1);
    }
    e.h().set_lifecycle(Lifecycle::Suspend);
    e.slow.fail.store(false, Ordering::SeqCst);
    e.h().set_lifecycle(Lifecycle::Resume);
    let c = e.wait_cycles(1, Duration::from_secs(3));
    assert!(c.len() == 1 && c[0].error.is_none(), "the wake sync runs");
}

#[test]
fn a_success_resets_the_failure_count() {
    let e = env_with(1, 3, Options { settle: Duration::from_millis(50), ..Options::default() });
    e.slow.fail.store(true, Ordering::SeqCst);
    assert_eq!(e.wait_cycles(1, Duration::from_secs(5)).len(), 1);
    e.h().set_lifecycle(Lifecycle::NetworkUp);
    assert_eq!(e.wait_cycles(1, Duration::from_secs(3)).len(), 1);
    e.slow.fail.store(false, Ordering::SeqCst);
    e.h().set_lifecycle(Lifecycle::NetworkUp);
    assert!(e.wait_cycles(1, Duration::from_secs(3))[0].error.is_none());
    e.slow.fail.store(true, Ordering::SeqCst);
    for _ in 0..2 {
        e.h().set_lifecycle(Lifecycle::NetworkUp);
        assert_eq!(e.wait_cycles(1, Duration::from_secs(3)).len(), 1);
    }
    e.h().set_lifecycle(Lifecycle::NetworkUp);
    assert_eq!(e.wait_cycles(1, Duration::from_secs(3)).len(), 1, "two failures since the success: still retrying");
}

#[test]
fn connections_that_never_come_count_as_failures() {
    let e = env_with(1, 3, Options { connect_timeout: Duration::from_millis(400), ..offline(WifiPolicy::TurnOn) });
    for i in 0..3 {
        e.h().submit(Command::Refill);
        let us = e.until(Duration::from_secs(3), |u| matches!(u, Update::Wifi(false)));
        assert_eq!(wifi_requests(&us), vec![true, false], "attempt {i}");
    }
    let us = e.until(Duration::from_millis(300), |u| matches!(u, Update::Status(_)));
    assert!(us.contains(&Update::Status(SyncStatus::Stopped)), "{us:?}");
    e.h().changed();
    std::thread::sleep(Duration::from_millis(2500));
    assert!(wifi_requests(&e.drain()).is_empty(), "stopped: local changes do not turn the Wi-Fi on");
    e.h().submit(Command::SyncNow);
    assert_eq!(wifi_requests(&e.until(Duration::from_secs(2), |u| matches!(u, Update::Wifi(true)))), vec![true], "a tap does");
}
