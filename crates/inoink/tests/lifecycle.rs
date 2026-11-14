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

/// Delegates to the fake server after a delay; counts requests; can be told to hang.
struct Slow {
    inner: Arc<Fake>,
    delay: Duration,
    requests: AtomicUsize,
    hang: AtomicBool,
}
impl Transport for Slow {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse> {
        self.requests.fetch_add(1, Ordering::SeqCst);
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
    });
    let clock = FixedClock::new(NOW);
    let tokens = Arc::new(MemTokens::default());
    *tokens.tokens.lock().unwrap() =
        Some(Tokens { access_token: "acc".into(), refresh_token: "ref".into(), expires_at: NOW + 3600, scope: String::new() });
    let engine = engine_for(dir.path(), &slow, &clock, &tokens);
    let (utx, urx) = channel();
    let path: PathBuf = dir.path().join("db.sqlite");
    let handle = start(engine, "42", Box::new(move || Store::open(&path, "42")), utx);
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
