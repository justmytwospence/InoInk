//! Application-owned InoInk state: the UI's database connection, the background sync handle
//! and what the status line shows. The UI only ever reads and writes the local database;
//! the network lives entirely on the sync worker.

use crate::settings::InoInkSettings;
use crate::view::Event;
use inoink::api::{ApiClient, SystemClock, UreqTransport};
use inoink::cache::{Cache, CacheConfig};
use inoink::content::images::UreqFetcher;
use inoink::credentials::{ensure_device_key, import_provisioned, FileCredentials};
use inoink::model::*;
use inoink::runtime::{self, Handle, Lifecycle, SyncStatus, Update};
use inoink::store::Store;
use inoink::sync::{SyncConfig, SyncEngine};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::time::Duration;

pub const DB_FILE: &str = "inoink.db";

pub struct InoInk {
    pub data_dir: PathBuf,
    pub account: String,
    /// The UI's connection. `None` while USB storage is shared.
    pub store: Option<Store>,
    /// The sync worker. `None` when not signed in (the app still reads what is cached).
    pub handle: Option<Handle>,
    pub status: SyncStatus,
    pub cache: Cache,
    pub collection: Collection,
    pub unread_only: bool,
    /// The most recent Done, offered for undo until superseded.
    pub last_undo: Option<(UndoId, String)>,
    /// A one-line message for the next redraw of the list.
    pub notice: Option<String>,
    /// An article waiting for its content to be fetched online.
    pub opening: Option<ArticleId>,
    pub settings: InoInkSettings,
    pub provision_error: Option<String>,
}

fn cache_config(s: &InoInkSettings) -> CacheConfig {
    CacheConfig { budget_bytes: s.cache_budget_mb * 1024 * 1024, recent_unread_days: s.recent_unread_days, pinned_folders: s.pinned_folders.clone(), ..Default::default() }
}

impl InoInk {
    /// Opens the data directory: imports provisioned credentials if present, opens the
    /// database and, when signed in, starts the sync worker. `hub` receives sync updates as
    /// [`Event::Sync`]. `online` is false for the emulator's offline fixtures.
    pub fn open(settings: &InoInkSettings, hub: Sender<Event>, online: bool) -> Result<InoInk, String> {
        let dir = settings.data_dir.clone();
        std::fs::create_dir_all(&dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
        let mut provision_error = None;
        if online {
            let _ = ensure_device_key(&dir);
            if let Err(e) = import_provisioned(&dir) {
                provision_error = Some(e.to_string());
            }
        }
        let creds_store = FileCredentials::new(&dir);
        let creds = creds_store.load_credentials().ok().flatten();
        let account = creds.as_ref().map(|c| c.user_id.clone()).filter(|u| !u.is_empty()).unwrap_or_else(|| "local".to_string());
        let db = dir.join(DB_FILE);
        let store = Store::open(&db, &account).map_err(|e| format!("can't open the database: {e}"))?;
        let cache = Cache::new(&dir.join("cache"), &account, cache_config(settings));
        let mut me = InoInk {
            data_dir: dir.clone(),
            account: account.clone(),
            store: Some(store),
            handle: None,
            status: SyncStatus::Idle { last_sync: None },
            cache,
            collection: Collection::Starred,
            unread_only: false,
            last_undo: None,
            notice: None,
            opening: None,
            settings: settings.clone(),
            provision_error,
        };
        me.load_ui_state();
        if let (true, Some(creds)) = (online, creds) {
            me.start_sync(creds.app_config(), creds_store, &db, hub)?;
        } else if online {
            me.status = SyncStatus::NeedsSignIn;
        }
        Ok(me)
    }

    fn start_sync(&mut self, cfg: inoink::api::AppConfig, creds: FileCredentials, db: &Path, hub: Sender<Event>) -> Result<(), String> {
        let transport = Arc::new(UreqTransport::new());
        let clock = Arc::new(SystemClock);
        let api = ApiClient::new(cfg.clone(), transport, clock.clone(), Arc::new(creds)).map_err(|e| e.to_string())?;
        let engine_store = Store::open(db, &self.account).map_err(|e| e.to_string())?;
        let cache = Cache::new(&self.data_dir.join("cache"), &self.account, cache_config(&self.settings));
        let fetcher = Arc::new(UreqFetcher::new(&cfg.user_agent));
        let free_dir = self.data_dir.clone();
        let engine = SyncEngine::new(engine_store, api, cache, fetcher, clock, SyncConfig::default())
            .with_free_space(Arc::new(move || inoink::cache::free_space_of(&free_dir)));
        let (utx, urx) = channel::<Update>();
        let account = self.account.clone();
        let db2 = db.to_path_buf();
        let reopen_account = account.clone();
        let handle = runtime::start(engine, &account, Box::new(move || Store::open(&db2, &reopen_account)), utx);
        std::thread::Builder::new()
            .name("inoink-updates".into())
            .spawn(move || {
                for u in urx {
                    if hub.send(Event::Sync(u)).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        self.handle = Some(handle);
        Ok(())
    }

    fn load_ui_state(&mut self) {
        let Some(store) = self.store.as_ref() else { return };
        if let Ok(Some(c)) = store.get_meta("ui_collection") {
            if let Ok(c) = serde_json::from_str::<Collection>(&c) {
                self.collection = c;
            }
        }
        if let Ok(Some(u)) = store.get_meta("ui_unread_only") {
            self.unread_only = u == "1";
        }
    }

    /// Remembers the collection and filter so the app reopens where it was left.
    pub fn set_view(&mut self, collection: Collection, unread_only: bool) {
        self.collection = collection;
        self.unread_only = unread_only;
        if let Some(store) = self.store.as_mut() {
            let _ = store.set_meta("ui_collection", &serde_json::to_string(&self.collection).unwrap_or_default());
            let _ = store.set_meta("ui_unread_only", if unread_only { "1" } else { "0" });
        }
    }

    pub fn store(&self) -> Option<&Store> {
        self.store.as_ref()
    }

    pub fn store_mut(&mut self) -> Option<&mut Store> {
        self.store.as_mut()
    }

    /// Tell the worker a local change was committed.
    pub fn changed(&self) {
        if let Some(h) = &self.handle {
            h.changed();
        }
    }

    pub fn submit(&self, cmd: runtime::Command) {
        if let Some(h) = &self.handle {
            h.submit(cmd);
        }
    }

    pub fn lifecycle(&self, l: Lifecycle) {
        if let Some(h) = &self.handle {
            h.set_lifecycle(l);
        }
    }

    /// Waits (bounded) for the worker to stop before suspend.
    pub fn quiesce(&self, deadline: Duration) -> bool {
        self.handle.as_ref().map(|h| h.quiesce(deadline)).unwrap_or(true)
    }

    /// USB mass storage is about to be shared: close every database handle.
    pub fn usb_share(&mut self) {
        self.lifecycle(Lifecycle::UsbShare);
        let _ = self.quiesce(Duration::from_secs(5));
        if let Some(s) = self.store.take() {
            let _ = s.checkpoint();
        }
    }

    pub fn usb_unshare(&mut self) {
        if self.store.is_none() {
            self.store = Store::open(&self.data_dir.join(DB_FILE), &self.account).ok();
        }
        self.lifecycle(Lifecycle::UsbUnshare);
    }

    pub fn pending(&self) -> usize {
        self.store.as_ref().and_then(|s| s.pending_count().ok()).unwrap_or(0)
    }

    /// Folds a worker update into the state. Returns true when lists should be redrawn.
    pub fn apply(&mut self, u: &Update) -> bool {
        match u {
            Update::Status(s) => {
                self.status = s.clone();
                false
            }
            Update::ArticlesChanged | Update::OrgChanged => true,
            Update::Cycle(rep) => {
                if !rep.cannot_unread.is_empty() {
                    self.notice = Some("Inoreader cannot mark this older article unread".into());
                }
                if rep.storage_paused {
                    self.notice = Some("Storage limit reached".into());
                }
                false
            }
            Update::Cached(..) | Update::CacheFailed(..) => true,
            _ => false,
        }
    }

    /// The status line shown in the list footer.
    pub fn status_text(&self) -> String {
        let pending = self.pending();
        let base = match &self.status {
            SyncStatus::Syncing => "Syncing".to_string(),
            SyncStatus::Idle { last_sync: Some(t) } => format!("Synced {}", clock_text(*t)),
            SyncStatus::Idle { last_sync: None } => {
                if self.handle.is_some() {
                    "Ready".to_string()
                } else {
                    "Offline library".to_string()
                }
            }
            SyncStatus::Offline => "Offline".to_string(),
            SyncStatus::QuotaPaused { .. } => "Paused: daily limit reached".to_string(),
            SyncStatus::NeedsSignIn => "Sign in required".to_string(),
            SyncStatus::Error(_) => "Sync problem".to_string(),
            SyncStatus::Paused => "Sync paused".to_string(),
        };
        if pending > 0 {
            format!("{base} \u{b7} {pending} to send")
        } else {
            base
        }
    }
}

fn clock_text(t: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_opt(t, 0).single().map(|d| d.format("%H:%M").to_string()).unwrap_or_default()
}
