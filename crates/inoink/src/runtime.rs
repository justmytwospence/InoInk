//! Background worker: owns the [`SyncEngine`] on its own thread, debounces local changes,
//! reacts to device lifecycle events and reports progress over a channel. Nothing here is
//! awaited by the UI: callers send commands and read updates.
//!
//! There is no periodic timer. Work happens only after a launch, an eligible wake or network
//! event, a user action or a committed local change, so the device is never kept awake.
//!
//! The worker knows whether the network is up ([`Lifecycle::NetworkUp`] and
//! [`Lifecycle::NetworkDown`]). A sync that comes due without it either waits for it
//! ([`WifiPolicy::Wait`]) or asks the app to turn the Wi-Fi on ([`Update::Wifi`]) and hands it
//! back when the sync ends ([`WifiPolicy::TurnOn`]). After [`MAX_FAILURES`] failed syncs in a row
//! the worker stops starting syncs on its own until the device wakes or the user asks for one.

use crate::error::Result;
use crate::model::*;
use crate::store::Store;
use crate::sync::{CycleReport, Reason, SyncEngine, SyncError};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Local changes are sent this long after the last one.
pub const DEBOUNCE: Duration = Duration::from_secs(2);
/// But never later than this after the first unsent change.
pub const MAX_DEBOUNCE: Duration = Duration::from_secs(10);
/// Network (re)connection syncs wait for the link to settle.
pub const NETWORK_SETTLE: Duration = Duration::from_secs(5);
/// A sync that asked for the Wi-Fi gives up when no connection comes within this.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
/// With [`WifiPolicy::TurnOn`], local changes turn the Wi-Fi on at most this often; other
/// syncs (launch, wake, the user's) connect at once.
pub const LOCAL_CONNECT_INTERVAL: Duration = Duration::from_secs(15 * 60);
/// After this many failed syncs in a row, automatic syncs stop until a wake or a sync the user
/// starts.
pub const MAX_FAILURES: u32 = 3;

/// What a sync does when it comes due while the Wi-Fi is off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WifiPolicy {
    /// Wait until the Wi-Fi is on; never turn it on.
    #[default]
    Wait,
    /// Turn the Wi-Fi on for the sync and off again afterwards, only when InoInk turned it on.
    TurnOn,
}

/// How the worker starts.
#[derive(Clone, Debug)]
pub struct Options {
    pub wifi: WifiPolicy,
    /// Whether the network is usable at start. The Kobo app starts without it and reports
    /// [`Lifecycle::NetworkUp`] once the Wi-Fi has an address.
    pub online: bool,
    pub settle: Duration,
    pub connect_timeout: Duration,
    pub local_connect_interval: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            wifi: WifiPolicy::Wait,
            online: true,
            settle: NETWORK_SETTLE,
            connect_timeout: CONNECT_TIMEOUT,
            local_connect_interval: LOCAL_CONNECT_INTERVAL,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifecycle {
    Suspend,
    Resume,
    NetworkUp,
    NetworkDown,
    /// USB mass storage is about to be shared: close the database.
    UsbShare,
    UsbUnshare,
}

#[derive(Debug)]
pub enum Command {
    /// A local change was committed to the store.
    Changed,
    SyncNow,
    /// After a bulk mark-read: send it and fetch now, regardless of the minimum interval,
    /// unless paused (quota) or sharing USB.
    Refill,
    Browse {
        collection: Collection,
        continuation: Option<String>,
    },
    CacheNow(ArticleId),
    RetryImage(ArticleId, String),
    ExtractOriginal(ArticleId),
    RefreshComments(ArticleId),
    /// Fetch a linked page and prepare its reading view.
    FetchPage(String),
    Lifecycle(Lifecycle),
    /// The Wi-Fi setting changed.
    SetWifiPolicy(WifiPolicy),
    Quiesce(Sender<()>),
    Shutdown,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SyncStatus {
    Idle {
        last_sync: Option<i64>,
    },
    Syncing,
    Offline,
    QuotaPaused {
        secs: u64,
    },
    NeedsSignIn,
    Error(String),
    Paused,
    /// Waiting for the Wi-Fi the worker asked for.
    Connecting,
    /// [`MAX_FAILURES`] syncs failed in a row: nothing starts on its own until a wake or a sync
    /// the user starts.
    Stopped,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    Status(SyncStatus),
    /// Article rows or states changed; redraw lists.
    ArticlesChanged,
    OrgChanged,
    Cycle(CycleReport),
    Browsed {
        collection: Collection,
        next: Option<String>,
    },
    BrowseFailed(String),
    Cached(ArticleId, DownloadState),
    CacheFailed(ArticleId, String),
    Extracted(ArticleId, std::result::Result<DownloadState, String>),
    CommentsRefreshed(ArticleId, std::result::Result<u32, String>),
    /// A linked page is ready (the file) or cannot be shown (why).
    PageReady(String, std::result::Result<std::path::PathBuf, String>),
    /// The worker stopped touching the database (USB) or the network (suspend).
    Quiesced,
    /// `true`: turn the Wi-Fi on for a sync ([`WifiPolicy::TurnOn`]). `false`: that sync is over;
    /// turn the Wi-Fi off again if InoInk turned it on.
    Wifi(bool),
    /// A sync the user tapped waits for the Wi-Fi ([`WifiPolicy::Wait`]).
    WaitingForWifi,
}

pub struct Handle {
    tx: Sender<Command>,
    cancel: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Handle {
    /// Queues work; never blocks.
    pub fn submit(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }

    /// Announces a committed local change (debounced into one sync).
    pub fn changed(&self) {
        self.submit(Command::Changed);
    }

    /// Delivers a lifecycle event. Suspend, network loss and USB sharing also raise the cancel
    /// flag immediately so an in-progress cycle stops at its next step instead of queueing
    /// behind it.
    pub fn set_lifecycle(&self, l: Lifecycle) {
        match l {
            Lifecycle::Suspend | Lifecycle::NetworkDown | Lifecycle::UsbShare => self.cancel.store(true, Ordering::Relaxed),
            _ => {}
        }
        self.submit(Command::Lifecycle(l));
    }

    /// Waits until the worker is idle (cancelled work finished) or `deadline` passes.
    /// Returns whether it quiesced in time. The caller must not hold database handles open
    /// across USB sharing regardless of the result.
    pub fn quiesce(&self, deadline: Duration) -> bool {
        let (tx, rx) = channel();
        self.submit(Command::Quiesce(tx));
        rx.recv_timeout(deadline).is_ok()
    }

    /// Stops the worker and waits for it.
    pub fn shutdown(mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.submit(Command::Shutdown);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // Dropping the sender ends the worker loop; do not join here (it may be mid-request).
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Reopens the database after USB sharing.
pub type Reopen = Box<dyn Fn() -> Result<Store> + Send>;

struct Worker {
    engine: SyncEngine,
    updates: Sender<Update>,
    reopen: Reopen,
    account: String,
    paused: bool,
    usb: bool,
    online: bool,
    pending: Option<(Reason, Instant)>,
    first_change: Option<Instant>,
    opts: Options,
    /// A sync that came due without a network and waits for one.
    waiting: Option<Reason>,
    /// The worker asked for the Wi-Fi and has not handed it back yet.
    holding: bool,
    /// When the Wi-Fi it asked for must have connected.
    connect_by: Option<Instant>,
    /// When it last asked for the Wi-Fi.
    last_connect: Option<Instant>,
    /// Failed syncs in a row.
    failures: u32,
    /// The pending sync is one the user tapped.
    tapped: bool,
}

fn status_of(rep: &CycleReport, last_sync: Option<i64>) -> SyncStatus {
    match &rep.error {
        None => SyncStatus::Idle { last_sync },
        Some(SyncError::Offline) => SyncStatus::Offline,
        Some(SyncError::NeedsSignIn) => SyncStatus::NeedsSignIn,
        Some(SyncError::QuotaPaused { secs }) => SyncStatus::QuotaPaused { secs: *secs },
        Some(SyncError::Other(m)) => SyncStatus::Error(m.clone()),
    }
}

/// The stronger of two reasons.
fn stronger(a: Reason, b: Reason) -> Reason {
    if rank(b) > rank(a) {
        b
    } else {
        a
    }
}

impl Worker {
    fn send(&self, u: Update) {
        let _ = self.updates.send(u);
    }

    fn stopped(&self) -> bool {
        self.failures >= MAX_FAILURES
    }

    fn schedule(&mut self, reason: Reason, after: Duration) {
        if self.paused || self.usb || (self.stopped() && reason != Reason::Explicit) {
            return;
        }
        let at = Instant::now() + after;
        self.pending = match self.pending {
            // A stronger or earlier request wins; otherwise keep the earlier deadline.
            Some((r, t)) => Some((if rank(reason) > rank(r) { reason } else { r }, t.min(at))),
            None => Some((reason, at)),
        };
    }

    /// The next moment the worker has something to do.
    fn deadline(&self) -> Option<Instant> {
        match (self.pending.map(|p| p.1), self.connect_by) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Runs whatever came due.
    fn tick(&mut self) {
        let now = Instant::now();
        if self.connect_by.is_some_and(|t| t <= now) {
            self.connect_failed();
        }
        if let Some((reason, at)) = self.pending {
            if at <= now {
                self.due(reason);
            }
        }
    }

    /// A sync came due: run it, or wait for the network (asking for the Wi-Fi if allowed).
    fn due(&mut self, reason: Reason) {
        self.pending = None;
        let tapped = std::mem::take(&mut self.tapped);
        if self.paused || self.usb || (self.stopped() && reason != Reason::Explicit) {
            self.release();
            return;
        }
        if self.online {
            self.run(reason);
            return;
        }
        match self.opts.wifi {
            WifiPolicy::Wait => {
                self.waiting = Some(self.waiting.map_or(reason, |w| stronger(w, reason)));
                self.first_change = None;
                if tapped {
                    self.send(Update::WaitingForWifi);
                }
                self.send(Update::Status(SyncStatus::Offline));
            }
            WifiPolicy::TurnOn => {
                let now = Instant::now();
                if reason == Reason::Local && !self.holding {
                    // Changes made offline go with the next connection; they turn the Wi-Fi on
                    // themselves only once the last connection is long enough ago.
                    if let Some(next) = self.last_connect.map(|t| t + self.opts.local_connect_interval).filter(|t| *t > now) {
                        self.pending = Some((Reason::Local, next));
                        return;
                    }
                }
                self.waiting = Some(self.waiting.map_or(reason, |w| stronger(w, reason)));
                self.first_change = None;
                if !self.holding {
                    self.holding = true;
                    self.last_connect = Some(now);
                    self.send(Update::Wifi(true));
                }
                if self.connect_by.is_none() {
                    self.connect_by = Some(now + self.opts.connect_timeout);
                }
                self.send(Update::Status(SyncStatus::Connecting));
            }
        }
    }

    /// The Wi-Fi the worker asked for did not connect in time.
    fn connect_failed(&mut self) {
        self.connect_by = None;
        self.waiting = None;
        self.failures += 1;
        self.release();
        self.send(Update::Status(if self.stopped() { SyncStatus::Stopped } else { SyncStatus::Offline }));
    }

    /// Called when no command is queued: the Wi-Fi the worker asked for is handed back once it
    /// is connected and no sync is left to run on it. Changes made during a pass are queued by
    /// then, so they go out on the same connection a few seconds later.
    fn idle(&mut self) {
        if self.holding && self.online && self.pending.is_none() {
            self.release();
        }
    }

    /// Hands back the Wi-Fi the worker asked for.
    fn release(&mut self) {
        self.connect_by = None;
        if self.holding {
            self.holding = false;
            self.send(Update::Wifi(false));
        }
    }

    fn run(&mut self, reason: Reason) {
        self.pending = None;
        self.first_change = None;
        self.waiting = None;
        if self.paused || self.usb {
            return;
        }
        self.send(Update::Status(SyncStatus::Syncing));
        let rep = self.engine.run_cycle(reason);
        match &rep.error {
            None => self.failures = 0,
            Some(SyncError::Offline) | Some(SyncError::Other(_)) => self.failures += 1,
            // Quota pauses and sign-in problems have their own state; retrying does not help.
            Some(_) => {}
        }

        let last = self.engine.store.get_meta("last_cycle").ok().flatten().and_then(|v| v.parse().ok());
        let changed = rep.sent_batches > 0
            || rep.fetched > 0
            || rep.cached > 0
            || !rep.cannot_unread.is_empty()
            || !rep.reconciled.is_empty()
            || rep.org_done > 0
            || rep.refreshed_org;
        let status = if self.paused {
            SyncStatus::Paused
        } else if self.stopped() && rep.error.is_some() {
            SyncStatus::Stopped
        } else {
            status_of(&rep, last)
        };
        self.send(Update::Status(status));
        if changed {
            self.send(Update::ArticlesChanged);
        }
        if rep.org_done > 0 || rep.refreshed_org {
            self.send(Update::OrgChanged);
        }
        self.send(Update::Cycle(rep));
    }

    fn handle(&mut self, cmd: Command) -> bool {
        match cmd {
            Command::Changed => {
                let now = Instant::now();
                let first = *self.first_change.get_or_insert(now);
                let after = DEBOUNCE.min((first + MAX_DEBOUNCE).saturating_duration_since(now));
                self.schedule(Reason::Local, after);
            }
            Command::SyncNow => {
                self.engine.cancel.store(false, Ordering::Relaxed);
                self.paused = false;
                self.failures = 0;
                self.tapped = true;
                self.pending = Some((Reason::Explicit, Instant::now()));
            }
            Command::SetWifiPolicy(p) => {
                self.opts.wifi = p;
                match p {
                    WifiPolicy::Wait => self.release(),
                    WifiPolicy::TurnOn => {
                        // A sync waiting for the network may now turn the Wi-Fi on.
                        if let (false, Some(w)) = (self.online, self.waiting.take()) {
                            self.pending = Some((self.pending.map_or(w, |(r, _)| stronger(r, w)), Instant::now()));
                        }
                    }
                }
            }
            Command::Refill => self.schedule(Reason::Explicit, Duration::ZERO),
            Command::Browse { collection, continuation } => {
                if self.usb {
                    self.send(Update::BrowseFailed("storage is being shared".into()));
                } else {
                    self.engine.cancel.store(false, Ordering::Relaxed);
                    match self.engine.browse(&collection, continuation) {
                        Ok(next) => {
                            self.send(Update::ArticlesChanged);
                            self.send(Update::Browsed { collection, next });
                        }
                        Err(e) => self.send(Update::BrowseFailed(e.to_string())),
                    }
                }
            }
            Command::CacheNow(id) => {
                if !self.usb {
                    match self.engine.cache_now(id) {
                        Ok(state) => self.send(Update::Cached(id, state)),
                        Err(e) => self.send(Update::CacheFailed(id, e.to_string())),
                    }
                }
            }
            Command::RetryImage(id, name) => {
                if !self.usb {
                    let ctl_cancel = self.engine.cancel.clone();
                    ctl_cancel.store(false, Ordering::Relaxed);
                    let r = {
                        let ctl = crate::cache::Control::new(ctl_cancel);
                        let fetcher = self.engine.fetcher();
                        // Split borrows: the cache lives in the engine alongside the store.
                        self.engine.retry_image(id, &name, &ctl, fetcher.as_ref())
                    };
                    match r {
                        Ok(state) => self.send(Update::Cached(id, state)),
                        Err(e) => self.send(Update::CacheFailed(id, e.to_string())),
                    }
                }
            }
            Command::ExtractOriginal(id) => {
                if !self.usb {
                    match self.engine.extract_original(id) {
                        Ok(r) => self.send(Update::Extracted(id, r)),
                        Err(e) => self.send(Update::Extracted(id, Err(e.to_string()))),
                    }
                }
            }
            Command::RefreshComments(id) => {
                if !self.usb {
                    let r = self.engine.refresh_comments(id).map_err(|e| e.to_string());
                    self.send(Update::CommentsRefreshed(id, r));
                }
            }
            Command::FetchPage(url) => {
                if !self.usb {
                    let r = self.engine.fetch_page(&url).unwrap_or_else(|e| Err(e.to_string()));
                    self.send(Update::PageReady(url, r));
                }
            }
            Command::Lifecycle(l) => self.lifecycle(l),
            Command::Quiesce(reply) => {
                let _ = self.engine.store.checkpoint();
                let _ = reply.send(());
                self.send(Update::Quiesced);
            }
            Command::Shutdown => {
                let _ = self.engine.store.checkpoint();
                return false;
            }
        }
        true
    }

    fn lifecycle(&mut self, l: Lifecycle) {
        match l {
            Lifecycle::Suspend => {
                self.paused = true;
                self.pending = None;
                // The app turns the Wi-Fi off for sleep itself.
                self.waiting = None;
                self.holding = false;
                self.connect_by = None;
                let _ = self.engine.store.checkpoint();
                self.send(Update::Status(SyncStatus::Paused));
            }
            Lifecycle::Resume => {
                self.paused = false;
                // A wake is a fresh start for the failure count.
                self.failures = 0;
                if self.online {
                    self.engine.cancel.store(false, Ordering::Relaxed);
                }
                self.send(Update::Status(SyncStatus::Idle { last_sync: None }));
                // Wi-Fi is usually not back yet; the network event normally triggers the sync.
                self.schedule(Reason::Wake, self.opts.settle);
            }
            Lifecycle::NetworkDown => {
                self.online = false;
                if matches!(self.pending, Some((Reason::Network, _)) | Some((Reason::Wake, _))) {
                    self.pending = None;
                }
                if self.holding {
                    // The Wi-Fi went off under the worker (the user switched it off): do not
                    // fight it; what waited goes with the next sync.
                    self.holding = false;
                    self.connect_by = None;
                    self.waiting = None;
                }
                // With TurnOn the Wi-Fi is off between syncs: keep showing the last sync.
                if self.opts.wifi == WifiPolicy::Wait {
                    self.send(Update::Status(SyncStatus::Offline));
                }
            }
            Lifecycle::NetworkUp => {
                self.online = true;
                self.connect_by = None;
                if !self.paused {
                    self.engine.cancel.store(false, Ordering::Relaxed);
                }
                // Flapping links coalesce: each event replaces the settle deadline. A sync that
                // waited for the network goes now; waiting local changes go with a full pass.
                let mut reason = Reason::Network;
                for r in [self.waiting.take(), self.pending.take().map(|p| p.0)].into_iter().flatten() {
                    reason = stronger(reason, if r == Reason::Local { Reason::Network } else { r });
                }
                self.schedule(reason, self.opts.settle);
            }
            Lifecycle::UsbShare => {
                self.usb = true;
                self.pending = None;
                self.waiting = None;
                self.release();
                let _ = self.engine.store.checkpoint();
                // Drop the file connection by swapping in a private in-memory store.
                if let Ok(mem) = Store::open_in_memory(&self.account) {
                    drop(std::mem::replace(&mut self.engine.store, mem));
                }
                self.send(Update::Quiesced);
            }
            Lifecycle::UsbUnshare => {
                if self.usb {
                    if let Ok(store) = (self.reopen)() {
                        self.engine.store = store;
                        let _ = self.engine.recover();
                        self.usb = false;
                        if !self.paused {
                            self.engine.cancel.store(false, Ordering::Relaxed);
                            self.schedule(Reason::Wake, self.opts.settle);
                        }
                    }
                }
            }
        }
    }
}

fn rank(r: Reason) -> u8 {
    match r {
        Reason::Wake => 0,
        Reason::Network => 1,
        Reason::Launch => 2,
        Reason::Local => 3,
        Reason::Explicit => 4,
    }
}

/// Starts the worker thread with the default [`Options`] (network up, [`WifiPolicy::Wait`]).
/// `reopen` must open the same database the engine's store uses.
pub fn start(engine: SyncEngine, account: &str, reopen: Reopen, updates: Sender<Update>) -> Handle {
    start_with(engine, account, reopen, updates, Options::default())
}

/// Starts the worker thread. `reopen` must open the same database the engine's store uses.
pub fn start_with(mut engine: SyncEngine, account: &str, reopen: Reopen, updates: Sender<Update>, opts: Options) -> Handle {
    let (tx, rx): (Sender<Command>, Receiver<Command>) = channel();
    let cancel = engine.cancel.clone();
    let account = account.to_string();
    let join = std::thread::Builder::new()
        .name("inoink-sync".into())
        .spawn(move || {
            let _ = engine.recover();
            let online = opts.online;
            let mut w = Worker {
                engine,
                updates,
                reopen,
                account,
                paused: false,
                usb: false,
                online,
                pending: None,
                first_change: None,
                opts,
                waiting: None,
                holding: false,
                connect_by: None,
                last_connect: None,
                failures: 0,
                tapped: false,
            };
            // Launch sync (gated by the minimum interval inside the engine).
            w.schedule(Reason::Launch, Duration::from_millis(0));
            loop {
                let cmd = match rx.try_recv() {
                    Ok(c) => Some(c),
                    Err(TryRecvError::Disconnected) => break,
                    Err(TryRecvError::Empty) => {
                        w.idle();
                        match w.deadline() {
                            Some(at) => match rx.recv_timeout(at.saturating_duration_since(Instant::now())) {
                                Ok(c) => Some(c),
                                Err(RecvTimeoutError::Timeout) => None,
                                Err(RecvTimeoutError::Disconnected) => break,
                            },
                            None => match rx.recv() {
                                Ok(c) => Some(c),
                                Err(_) => break,
                            },
                        }
                    }
                };
                match cmd {
                    Some(c) => {
                        if !w.handle(c) {
                            break;
                        }
                    }
                    None => w.tick(),
                }
            }
        })
        .expect("spawn sync worker");
    Handle { tx, cancel, join: Some(join) }
}
