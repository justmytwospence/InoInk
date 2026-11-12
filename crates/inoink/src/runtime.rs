//! Background worker: owns the [`SyncEngine`] on its own thread, debounces local changes,
//! reacts to device lifecycle events and reports progress over a channel. Nothing here is
//! awaited by the UI: callers send commands and read updates.
//!
//! There is no periodic timer. Work happens only after a launch, an eligible wake or network
//! event, a user action or a committed local change, so the device is never kept awake.

use crate::error::Result;
use crate::model::*;
use crate::store::Store;
use crate::sync::{CycleReport, Reason, SyncEngine, SyncError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Local changes are sent this long after the last one.
pub const DEBOUNCE: Duration = Duration::from_secs(2);
/// But never later than this after the first unsent change.
pub const MAX_DEBOUNCE: Duration = Duration::from_secs(10);
/// Network (re)connection syncs wait for the link to settle.
pub const NETWORK_SETTLE: Duration = Duration::from_secs(5);

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
    Browse {
        collection: Collection,
        continuation: Option<String>,
    },
    CacheNow(ArticleId),
    RetryImage(ArticleId, String),
    ExtractOriginal(ArticleId),
    RefreshComments(ArticleId),
    Lifecycle(Lifecycle),
    Quiesce(Sender<()>),
    Shutdown,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SyncStatus {
    Idle { last_sync: Option<i64> },
    Syncing,
    Offline,
    QuotaPaused { secs: u64 },
    NeedsSignIn,
    Error(String),
    Paused,
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
    /// The worker stopped touching the database (USB) or the network (suspend).
    Quiesced,
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

impl Worker {
    fn send(&self, u: Update) {
        let _ = self.updates.send(u);
    }

    fn schedule(&mut self, reason: Reason, after: Duration) {
        if self.paused || self.usb {
            return;
        }
        let at = Instant::now() + after;
        self.pending = match self.pending {
            // A stronger or earlier request wins; otherwise keep the earlier deadline.
            Some((r, t)) => Some((if rank(reason) > rank(r) { reason } else { r }, t.min(at))),
            None => Some((reason, at)),
        };
    }

    fn run(&mut self, reason: Reason) {
        self.pending = None;
        self.first_change = None;
        if self.paused || self.usb {
            return;
        }
        self.send(Update::Status(SyncStatus::Syncing));
        let rep = self.engine.run_cycle(reason);
        let last = self.engine.store.get_meta("last_cycle").ok().flatten().and_then(|v| v.parse().ok());
        let changed = rep.sent_batches > 0
            || rep.fetched > 0
            || rep.cached > 0
            || !rep.cannot_unread.is_empty()
            || !rep.reconciled.is_empty()
            || rep.org_done > 0
            || rep.refreshed_org;
        self.send(Update::Status(if self.paused { SyncStatus::Paused } else { status_of(&rep, last) }));
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
                self.pending = Some((Reason::Explicit, Instant::now()));
            }
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
                let _ = self.engine.store.checkpoint();
                self.send(Update::Status(SyncStatus::Paused));
            }
            Lifecycle::Resume => {
                self.paused = false;
                if self.online {
                    self.engine.cancel.store(false, Ordering::Relaxed);
                }
                self.send(Update::Status(SyncStatus::Idle { last_sync: None }));
                // Wi-Fi is usually not back yet; the network event normally triggers the sync.
                self.schedule(Reason::Wake, NETWORK_SETTLE);
            }
            Lifecycle::NetworkDown => {
                self.online = false;
                if matches!(self.pending, Some((Reason::Network, _)) | Some((Reason::Wake, _))) {
                    self.pending = None;
                }
                self.send(Update::Status(SyncStatus::Offline));
            }
            Lifecycle::NetworkUp => {
                self.online = true;
                if !self.paused {
                    self.engine.cancel.store(false, Ordering::Relaxed);
                }
                // Flapping links coalesce: each event replaces the settle deadline.
                self.pending = None;
                self.schedule(Reason::Network, NETWORK_SETTLE);
            }
            Lifecycle::UsbShare => {
                self.usb = true;
                self.pending = None;
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
                            self.schedule(Reason::Wake, NETWORK_SETTLE);
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

/// Starts the worker thread. `reopen` must open the same database the engine's store uses.
pub fn start(mut engine: SyncEngine, account: &str, reopen: Reopen, updates: Sender<Update>) -> Handle {
    let (tx, rx): (Sender<Command>, Receiver<Command>) = channel();
    let cancel = engine.cancel.clone();
    let account = account.to_string();
    let join = std::thread::Builder::new()
        .name("inoink-sync".into())
        .spawn(move || {
            let _ = engine.recover();
            let mut w =
                Worker { engine, updates, reopen, account, paused: false, usb: false, online: true, pending: None, first_change: None };
            // Launch sync (gated by the minimum interval inside the engine).
            w.schedule(Reason::Launch, Duration::from_millis(0));
            loop {
                let cmd = match w.pending {
                    Some((_, at)) => match rx.recv_timeout(at.saturating_duration_since(Instant::now())) {
                        Ok(c) => Some(c),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => break,
                    },
                    None => match rx.recv() {
                        Ok(c) => Some(c),
                        Err(_) => break,
                    },
                };
                match cmd {
                    Some(c) => {
                        if !w.handle(c) {
                            break;
                        }
                    }
                    None => {
                        if let Some((reason, _)) = w.pending {
                            w.run(reason);
                        }
                    }
                }
            }
        })
        .expect("spawn sync worker");
    Handle { tx, cancel, join: Some(join) }
}
