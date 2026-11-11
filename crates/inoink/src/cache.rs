//! Content cache: rooted files, atomic publication, recovery, download policy and eviction.
//!
//! Layout per account: `<root>/<account-hash>/<article-hex>/{v<N>.html, assets/<name>.<ext>}`.
//! The HTML engine is rooted at the article directory, so `assets/...` is the only kind of
//! reference a document can contain and nothing outside it can be reached.

use crate::content::images::{download_image, DownloadError, Fetcher, ImageLimits};
use crate::content::{asset_name, prepare_article, ArticleMeta, AssetMap, AssetState};
use crate::error::{Error, Result};
use crate::model::*;
use crate::store::{CandidateKind, Store};
use crate::util::{now_secs, sha256_hex};
use std::collections::{HashSet, VecDeque};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct CacheConfig {
    pub budget_bytes: u64,
    pub recent_unread_days: i64,
    /// Refuse to start downloads when less than this is free on the device.
    pub min_free_bytes: u64,
    pub limits: ImageLimits,
    pub concurrency: usize,
    pub max_candidates_per_cycle: usize,
    pub pinned_folders: Vec<String>,
}

impl Default for CacheConfig {
    fn default() -> Self {
        CacheConfig {
            budget_bytes: 512 * 1024 * 1024,
            recent_unread_days: 30,
            min_free_bytes: 64 * 1024 * 1024,
            limits: ImageLimits::default(),
            concurrency: 2,
            max_candidates_per_cycle: 50,
            pinned_folders: Vec::new(),
        }
    }
}

/// Per-call controls.
pub struct Control {
    pub cancel: Arc<AtomicBool>,
    pub sleep: Box<dyn Fn(Duration) + Send + Sync>,
    /// Free bytes on the cache volume, if known.
    pub free_space: Box<dyn Fn() -> Option<u64> + Send + Sync>,
}

impl Control {
    pub fn new(cancel: Arc<AtomicBool>) -> Control {
        Control { cancel, sleep: Box::new(std::thread::sleep), free_space: Box::new(|| None) }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DownloadPlan {
    pub ids: Vec<ArticleId>,
    /// Set when automatic downloads are paused ("Storage limit reached").
    pub paused: Option<PauseReason>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseReason {
    StorageLimit,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EvictionReport {
    pub evicted: Vec<ArticleId>,
    pub freed_bytes: u64,
    /// Protected content alone exceeds the budget (nothing more can be evicted).
    pub protected_over_budget: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecoveryReport {
    pub removed_partial: usize,
    pub removed_orphans: usize,
    pub dropped_missing_versions: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VerifyReport {
    pub checked: usize,
    pub problems: Vec<String>,
}

pub struct Cache {
    dir: PathBuf,
    cfg: CacheConfig,
}

fn io_err(e: std::io::Error) -> Error {
    if e.kind() == std::io::ErrorKind::StorageFull || e.raw_os_error() == Some(28) {
        Error::Storage("storage full".into())
    } else {
        Error::Io(e.to_string())
    }
}

/// Writes `data` to `path` atomically: write a `.part` sibling, fsync, rename, fsync the directory.
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let dir = path.parent().ok_or_else(|| Error::Io("no parent".into()))?;
    fs::create_dir_all(dir).map_err(io_err)?;
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let result = (|| {
        let mut f = File::create(&part)?;
        f.write_all(data)?;
        f.sync_all()?;
        fs::rename(&part, path)?;
        if let Ok(d) = File::open(dir) {
            let _ = d.sync_all();
        }
        Ok(())
    })();
    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&part);
            Err(io_err(e))
        }
    }
}

/// `YYYY-MM-DD` (UTC) for a unix timestamp.
pub fn format_date(secs: i64) -> String {
    if secs <= 0 {
        return String::new();
    }
    let days = secs.div_euclid(86400) + 719468;
    let era = days.div_euclid(146097);
    let doe = days.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Free bytes on the volume holding `path`.
pub fn free_space_of(path: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `st` is a properly sized out parameter.
    let rc = unsafe { libc::statvfs(c.as_ptr(), &mut st) };
    if rc == 0 {
        Some(st.f_bavail as u64 * st.f_frsize as u64)
    } else {
        None
    }
}

impl Cache {
    /// `root` is the cache directory; `account` selects the per-account subdirectory.
    pub fn new(root: &Path, account: &str, cfg: CacheConfig) -> Cache {
        let dir = root.join(&sha256_hex(account.as_bytes())[..16]);
        Cache { dir, cfg }
    }

    pub fn config(&self) -> &CacheConfig {
        &self.cfg
    }

    pub fn root(&self) -> &Path {
        &self.dir
    }

    /// Article directory (the HTML engine's document root).
    pub fn article_dir(&self, id: ArticleId) -> PathBuf {
        self.dir.join(format!("{:016x}", id.0))
    }

    pub fn version_path(&self, id: ArticleId, version: i64) -> PathBuf {
        self.article_dir(id).join(format!("v{version}.html"))
    }

    fn rel_dir(&self, id: ArticleId) -> String {
        format!("{:016x}", id.0)
    }

    fn asset_map(&self, store: &Store, id: ArticleId) -> Result<AssetMap> {
        let mut map = AssetMap::new();
        for a in store.assets(id)? {
            let st = match a.state.as_str() {
                "done" => {
                    let file = self.find_asset_file(id, &a.name);
                    match file {
                        Some(f) => AssetState::Done(f),
                        None => AssetState::Pending,
                    }
                }
                "failed" => AssetState::Failed,
                _ => AssetState::Pending,
            };
            map.insert(a.name, st);
        }
        Ok(map)
    }

    fn find_asset_file(&self, id: ArticleId, name: &str) -> Option<String> {
        for ext in ["png", "jpg"] {
            let f = format!("{name}.{ext}");
            if self.article_dir(id).join("assets").join(&f).is_file() {
                return Some(f);
            }
        }
        None
    }

    fn check_space(&self, ctl: &Control) -> Result<()> {
        if let Some(free) = (ctl.free_space)() {
            if free < self.cfg.min_free_bytes {
                return Err(Error::Storage("storage limit reached".into()));
            }
        }
        Ok(())
    }

    fn publish(
        &self,
        store: &mut Store,
        id: ArticleId,
        prepared: &crate::content::PreparedArticle,
        raw_hash: &str,
        title: &str,
        state: DownloadState,
    ) -> Result<Option<i64>> {
        let hash = sha256_hex(prepared.html.as_bytes());
        if let Some(latest) = store.latest_version(id)? {
            if latest.hash == hash && self.version_path(id, latest.version).is_file() {
                store.set_download_state(id, state)?;
                return Ok(None);
            }
        }
        let next = store.versions(id)?.last().map(|v| v.version).unwrap_or(0) + 1;
        let path = self.version_path(id, next);
        // File first, database second: the database never refers to a missing file.
        atomic_write(&path, prepared.html.as_bytes())?;
        store.publish_version(id, raw_hash, &hash, &self.rel_dir(id), prepared.html.len() as i64, title, &prepared.text, state)?;
        Ok(Some(next))
    }

    /// Caches one article: publishes readable text immediately, then downloads its images and
    /// publishes a final version. Returns the resulting state.
    pub fn cache_article(&self, store: &mut Store, fetcher: &dyn Fetcher, id: ArticleId, ctl: &Control) -> Result<DownloadState> {
        self.check_space(ctl)?;
        let rec = store.article_record(id)?.ok_or_else(|| Error::InvalidRequest("unknown article".into()))?;
        let base = if rec.canonical_url.is_empty() { None } else { Some(rec.canonical_url.clone()) };
        let raw_hash = sha256_hex(rec.summary_html.as_bytes());
        self.process(store, fetcher, id, &rec, &rec.summary_html, base.as_deref(), &raw_hash, ctl)
    }

    /// Prepares `html` for `rec`, publishes it, downloads missing images and republishes.
    #[allow(clippy::too_many_arguments)]
    fn process(
        &self,
        store: &mut Store,
        fetcher: &dyn Fetcher,
        id: ArticleId,
        rec: &crate::store::ArticleRecord,
        html: &str,
        base: Option<&str>,
        raw_hash: &str,
        ctl: &Control,
    ) -> Result<DownloadState> {
        let meta = ArticleMeta {
            title: rec.title.clone(),
            source: rec.origin_title.clone(),
            author: rec.author.clone(),
            date: format_date(rec.published),
            url: rec.canonical_url.clone(),
        };
        let mut map = self.asset_map(store, id)?;
        let prepared = prepare_article(html, base, &meta, &map);
        for a in &prepared.assets {
            if !map.contains_key(&a.name) {
                store.set_asset(id, &a.name, &a.url, "pending", 0)?;
                map.insert(a.name.clone(), AssetState::Pending);
            }
        }
        let all_done = |m: &AssetMap, p: &crate::content::PreparedArticle| {
            p.assets.iter().all(|a| matches!(m.get(&a.name), Some(AssetState::Done(_))))
        };
        let first_state = if all_done(&map, &prepared) { DownloadState::Complete } else { DownloadState::TextReady };
        self.publish(store, id, &prepared, raw_hash, &rec.title, first_state)?;

        let jobs: Vec<(String, String)> = prepared
            .assets
            .iter()
            .filter(|a| matches!(map.get(&a.name), Some(AssetState::Pending)))
            .map(|a| (a.name.clone(), a.url.clone()))
            .collect();
        if jobs.is_empty() {
            return self.final_state(store, id, &prepared, &map);
        }
        let changed = self.download_assets(store, fetcher, id, jobs, ctl)?;
        if changed {
            let map = self.asset_map(store, id)?;
            let prepared = prepare_article(html, base, &meta, &map);
            let state = self.final_state(store, id, &prepared, &map)?;
            self.publish(store, id, &prepared, raw_hash, &rec.title, state)?;
            self.collect_garbage(store, id)?;
            return Ok(state);
        }
        self.final_state(store, id, &prepared, &map)
    }

    fn final_state(
        &self,
        store: &mut Store,
        id: ArticleId,
        prepared: &crate::content::PreparedArticle,
        map: &AssetMap,
    ) -> Result<DownloadState> {
        let mut pending = false;
        let mut failed = false;
        for a in &prepared.assets {
            match map.get(&a.name) {
                Some(AssetState::Done(_)) => {}
                Some(AssetState::Failed) => failed = true,
                _ => pending = true,
            }
        }
        let state = if pending {
            DownloadState::TextReady
        } else if failed {
            DownloadState::MissingImages
        } else {
            DownloadState::Complete
        };
        store.set_download_state(id, state)?;
        Ok(state)
    }

    /// Downloads images with bounded concurrency. Returns whether any asset changed state.
    fn download_assets(
        &self,
        store: &mut Store,
        fetcher: &dyn Fetcher,
        id: ArticleId,
        jobs: Vec<(String, String)>,
        ctl: &Control,
    ) -> Result<bool> {
        let queue = Mutex::new(VecDeque::from(jobs));
        let (tx, rx) = mpsc::channel::<(String, String, std::result::Result<(String, u64), DownloadError>)>();
        let assets_dir = self.article_dir(id).join("assets");
        let limits = self.cfg.limits.clone();
        let mut changed = false;
        let mut hard_error: Option<Error> = None;
        std::thread::scope(|scope| {
            for _ in 0..self.cfg.concurrency.clamp(1, 4) {
                let tx = tx.clone();
                let queue = &queue;
                let assets_dir = &assets_dir;
                let limits = &limits;
                scope.spawn(move || loop {
                    if ctl.cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let job = queue.lock().unwrap().pop_front();
                    let Some((name, url)) = job else { break };
                    let result = match url::Url::parse(&url) {
                        Err(_) => Err(DownloadError::Permanent("bad URL".into())),
                        Ok(u) => download_image(fetcher, &u, limits, &ctl.cancel, &*ctl.sleep).and_then(|n| {
                            let file = format!("{name}.{}", n.ext);
                            atomic_write(&assets_dir.join(&file), &n.bytes).map(|_| (file, n.bytes.len() as u64)).map_err(|e| match e {
                                Error::Storage(m) => DownloadError::Transient(m),
                                e => DownloadError::Permanent(e.to_string()),
                            })
                        }),
                    };
                    if tx.send((name, url, result)).is_err() {
                        break;
                    }
                });
            }
            drop(tx);
            for (name, url, result) in rx.iter() {
                let r = match result {
                    Ok((_file, bytes)) => {
                        changed = true;
                        store.set_asset(id, &name, &url, "done", bytes as i64)
                    }
                    Err(DownloadError::Permanent(_)) => {
                        changed = true;
                        store.set_asset(id, &name, &url, "failed", 0)
                    }
                    // Transient or cancelled: stays pending and is retried later.
                    Err(_) => Ok(()),
                };
                if let Err(e) = r {
                    hard_error = Some(e);
                    ctl.cancel.store(true, Ordering::Relaxed);
                }
            }
        });
        match hard_error {
            Some(e) => Err(e),
            None => Ok(changed),
        }
    }

    /// Retries one image the user asked for, including permanently failed ones.
    pub fn retry_asset(&self, store: &mut Store, fetcher: &dyn Fetcher, id: ArticleId, name: &str, ctl: &Control) -> Result<DownloadState> {
        let row = store.assets(id)?.into_iter().find(|a| a.name == name).ok_or_else(|| Error::InvalidRequest("unknown image".into()))?;
        store.set_asset(id, name, &row.url, "pending", 0)?;
        self.cache_article(store, fetcher, id, ctl)
    }

    /// Removes superseded versions that no reader holds a lease on.
    pub fn collect_garbage(&self, store: &mut Store, id: ArticleId) -> Result<()> {
        if store.is_leased(id)? {
            return Ok(());
        }
        let versions = store.versions(id)?;
        if let Some(latest) = versions.last().map(|v| v.version) {
            for v in versions.iter().filter(|v| v.version != latest) {
                let _ = fs::remove_file(self.version_path(id, v.version));
                store.delete_version(id, v.version)?;
            }
        }
        Ok(())
    }

    /// Publishes extracted original-page HTML as a new version (the API source is untouched),
    /// downloading its images like any other article.
    pub fn publish_extracted(
        &self,
        store: &mut Store,
        fetcher: &dyn Fetcher,
        id: ArticleId,
        html: &str,
        page_url: &str,
        ctl: &Control,
    ) -> Result<DownloadState> {
        self.check_space(ctl)?;
        let rec = store.article_record(id)?.ok_or_else(|| Error::InvalidRequest("unknown article".into()))?;
        let raw_hash = format!("extract:{}", sha256_hex(html.as_bytes()));
        self.process(store, fetcher, id, &rec, html, Some(page_url), &raw_hash, ctl)
    }

    // ---- policy ---------------------------------------------------------------------------

    pub fn plan_downloads(&self, store: &mut Store, now: i64) -> Result<DownloadPlan> {
        store.sync_pins(&self.cfg.pinned_folders)?;
        let (_, protected_bytes) = self.usage(store)?;
        if protected_bytes >= self.cfg.budget_bytes {
            return Ok(DownloadPlan { ids: vec![], paused: Some(PauseReason::StorageLimit) });
        }
        let cutoff = (now - self.cfg.recent_unread_days * 86400) * 1_000_000;
        let ids = store
            .download_candidates(cutoff, self.cfg.max_candidates_per_cycle)?
            .into_iter()
            .map(|(id, _): (ArticleId, CandidateKind)| id)
            .collect();
        Ok(DownloadPlan { ids, paused: None })
    }

    /// (total cached bytes, bytes of protected articles)
    fn usage(&self, store: &Store) -> Result<(u64, u64)> {
        let mut total = 0u64;
        let mut protected = 0u64;
        for e in store.cache_usage()? {
            total += e.bytes.max(0) as u64;
            if e.starred || e.pinned || e.has_pending || e.leased {
                protected += e.bytes.max(0) as u64;
            }
        }
        Ok((total, protected))
    }

    /// Evicts the oldest unprotected articles until the budget is met. Starred, pinned-folder,
    /// leased and pending-action articles are never evicted.
    pub fn evict_to_budget(&self, store: &mut Store) -> Result<EvictionReport> {
        let mut report = EvictionReport::default();
        let entries = store.cache_usage()?;
        let mut total: u64 = entries.iter().map(|e| e.bytes.max(0) as u64).sum();
        for e in entries {
            if total <= self.cfg.budget_bytes {
                break;
            }
            if e.starred || e.pinned || e.has_pending || e.leased {
                continue;
            }
            let _ = fs::remove_dir_all(self.article_dir(e.article));
            store.clear_content(e.article)?;
            total = total.saturating_sub(e.bytes.max(0) as u64);
            report.freed_bytes += e.bytes.max(0) as u64;
            report.evicted.push(e.article);
        }
        report.protected_over_budget = total > self.cfg.budget_bytes;
        Ok(report)
    }

    // ---- recovery and verification --------------------------------------------------------

    /// Startup recovery after a crash or power loss: remove partial writes and files the
    /// database does not know, and forget versions whose files are gone (so the UI never claims
    /// an article is offline-ready when it is not).
    pub fn recover(&self, store: &mut Store) -> Result<RecoveryReport> {
        let mut rep = RecoveryReport::default();
        store.clear_leases()?;
        let Ok(rd) = fs::read_dir(&self.dir) else {
            return Ok(rep);
        };
        for entry in rd.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let Some(id) = entry.file_name().to_str().and_then(|n| u64::from_str_radix(n, 16).ok()).map(ArticleId) else {
                continue;
            };
            let versions = store.versions(id)?;
            let known: HashSet<String> = versions.iter().map(|v| format!("v{}.html", v.version)).collect();
            let done: HashSet<String> = store.assets(id)?.into_iter().filter(|a| a.state == "done").map(|a| a.name).collect();
            for f in fs::read_dir(&dir).into_iter().flatten().flatten() {
                let name = f.file_name().to_string_lossy().into_owned();
                let path = f.path();
                if name.ends_with(".part") {
                    let _ = fs::remove_file(&path);
                    rep.removed_partial += 1;
                } else if name.starts_with('v') && name.ends_with(".html") && !known.contains(&name) {
                    let _ = fs::remove_file(&path);
                    rep.removed_orphans += 1;
                } else if name == "assets" && path.is_dir() {
                    for a in fs::read_dir(&path).into_iter().flatten().flatten() {
                        let an = a.file_name().to_string_lossy().into_owned();
                        let stem = an.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or_default();
                        if an.ends_with(".part") {
                            let _ = fs::remove_file(a.path());
                            rep.removed_partial += 1;
                        } else if !done.contains(&stem) {
                            let _ = fs::remove_file(a.path());
                            rep.removed_orphans += 1;
                        }
                    }
                }
            }
        }
        for id in store.downloaded_articles()? {
            let versions = store.versions(id)?;
            for v in &versions {
                if !self.version_path(id, v.version).is_file() {
                    store.delete_version(id, v.version)?;
                    rep.dropped_missing_versions += 1;
                }
            }
            if store.versions(id)?.is_empty() {
                store.clear_content(id)?;
            }
        }
        Ok(rep)
    }

    /// Checks every published version (file present, hash matches) and every done asset.
    pub fn verify(&self, store: &Store) -> Result<VerifyReport> {
        let mut rep = VerifyReport::default();
        for id in store.downloaded_articles()? {
            let Some(v) = store.latest_version(id)? else {
                rep.problems.push(format!("{:016x}: marked downloaded but has no version", id.0));
                continue;
            };
            rep.checked += 1;
            match fs::read(self.version_path(id, v.version)) {
                Ok(bytes) if sha256_hex(&bytes) == v.hash => {}
                Ok(_) => rep.problems.push(format!("{:016x}: v{} hash mismatch", id.0, v.version)),
                Err(_) => rep.problems.push(format!("{:016x}: v{} file missing", id.0, v.version)),
            }
            for a in store.assets(id)? {
                if a.state == "done" && self.find_asset_file(id, &a.name).is_none() {
                    rep.problems.push(format!("{:016x}: asset {} missing", id.0, a.name));
                }
            }
        }
        Ok(rep)
    }
}

/// Convenience used by tests and the sync worker.
pub fn asset_name_for(url: &str) -> String {
    asset_name(url)
}

pub fn now() -> i64 {
    now_secs()
}
