//! Content cache: rooted files, atomic publication, recovery, download policy and eviction.
//!
//! Layout per account: `<root>/<account-hash>/<article-hex>/{v<N>.html, assets/<name>.<ext>}`.
//! The HTML engine is rooted at the article directory, so `assets/...` is the only kind of
//! reference a document can contain and nothing outside it can be reached.

use crate::content::discussion::{self, Discussion, Site, Thread};
use crate::content::extract::{extract_main, extract_original, page_title, Extraction, MIN_TEXT_CHARS};
use crate::content::images::{download_image, fetch_following, DownloadError, Fetcher, ImageLimits};
use crate::content::math;
use crate::content::sanitize::{document, sanitize_fragment};
use crate::content::{asset_name, prepare_article, ArticleMeta, AssetMap, AssetState};
use crate::error::{Error, Result};
use crate::model::*;
use crate::store::{CandidateKind, Store};
use crate::util::{now_secs, sha256_hex};
use std::collections::{HashSet, VecDeque};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

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
    /// Fetch the original page for teaser items (see `content::stub`).
    pub extract_stubs: bool,
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
            extract_stubs: false,
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

/// Civil date (year, month, day) of a Unix time in UTC.
pub(crate) fn civil(secs: i64) -> (i64, i64, i64) {
    let days = secs.div_euclid(86400) + 719468;
    let era = days.div_euclid(146097);
    let doe = days.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `YYYY-MM-DD` (UTC) for a unix timestamp.
pub fn format_date(secs: i64) -> String {
    if secs <= 0 {
        return String::new();
    }
    let (y, m, d) = civil(secs);
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
                "done" => match self.find_asset_file(id, &a.name) {
                    Some(file) if math::is_math_url(&a.url) => match self.read_math_metrics(id, &a.name) {
                        Some((width_em, depth_em)) => AssetState::Formula { file, width_em, depth_em },
                        None => AssetState::Pending,
                    },
                    Some(f) => AssetState::Done(f),
                    None => AssetState::Pending,
                },
                "failed" => AssetState::Failed,
                _ => AssetState::Pending,
            };
            map.insert(a.name, st);
        }
        Ok(map)
    }

    /// Size and baseline depth (in em) of a typeset formula, from its `.math` sidecar.
    fn read_math_metrics(&self, id: ArticleId, name: &str) -> Option<(f32, f32)> {
        let text = fs::read_to_string(self.article_dir(id).join("assets").join(format!("{name}.math"))).ok()?;
        let mut it = text.split_whitespace().map(|v| v.parse::<f32>().ok().filter(|f| f.is_finite() && *f >= 0.0));
        Some((it.next()??, it.next()??))
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
        if let Some(d) = discussion::detect(&rec.canonical_url, &rec.summary_html) {
            return self.cache_link_post(store, fetcher, id, &rec, &d, ctl);
        }
        if self.cfg.extract_stubs && rec.canonical_url.starts_with("https://") {
            if let Some(state) = self.try_full_text(store, fetcher, id, &rec, ctl)? {
                return Ok(state);
            }
        }
        let base = if rec.canonical_url.is_empty() { None } else { Some(rec.canonical_url.clone()) };
        let raw_hash = sha256_hex(rec.summary_html.as_bytes());
        self.process(store, fetcher, id, &rec, &rec.summary_html, base.as_deref(), &raw_hash, ctl)
    }

    /// For a teaser item (or a feed known to send teasers): the original page's text in place of
    /// the feed's. `None` when the feed text stays, because the page was not tried, could not be
    /// fetched, or had no more text. What each try showed is kept per feed.
    fn try_full_text(
        &self,
        store: &mut Store,
        fetcher: &dyn Fetcher,
        id: ArticleId,
        rec: &crate::store::ArticleRecord,
        ctl: &Control,
    ) -> Result<Option<DownloadState>> {
        use crate::content::stub::{looks_like_stub, should_extract};
        let feed_text = prepare_article(&rec.summary_html, None, &ArticleMeta::default(), &AssetMap::new()).text;
        let ff = store.feed_full_text(&rec.origin_stream)?;
        if !should_extract(ff.mode, ff.tries, ff.gains, looks_like_stub(&feed_text)) {
            return Ok(None);
        }
        match extract_original(fetcher, &rec.canonical_url, feed_text.chars().count(), &self.cfg.limits, &ctl.cancel) {
            Ok(Extraction::Extracted { html, .. }) => {
                store.record_full_text_try(&rec.origin_stream, true)?;
                let raw_hash = format!("extract:{}", sha256_hex(html.as_bytes()));
                self.process(store, fetcher, id, rec, &html, Some(&rec.canonical_url), &raw_hash, ctl).map(Some)
            }
            Ok(Extraction::Failed(_)) => {
                store.record_full_text_try(&rec.origin_stream, false)?;
                Ok(None)
            }
            Err(Error::Storage(m)) => Err(Error::Storage(m)),
            // Offline or a transient failure: the feed text stands, and nothing is learned.
            Err(_) => Ok(None),
        }
    }

    pub fn comments_path(&self, id: ArticleId) -> PathBuf {
        self.article_dir(id).join("comments.html")
    }

    fn fetch_thread(&self, fetcher: &dyn Fetcher, d: &Discussion, ctl: &Control) -> Result<Thread> {
        let url = url::Url::parse(&d.api_url()).map_err(|_| Error::InvalidRequest("bad discussion address".into()))?;
        let fetched = fetch_following(fetcher, &url, &self.cfg.limits, &ctl.cancel)?;
        discussion::parse_thread(d.site, &fetched.body)
    }

    fn write_comments(&self, store: &mut Store, id: ArticleId, d: &Discussion, title: &str, thread: &Thread) -> Result<u32> {
        let page = discussion::render_comments(thread, d, title);
        fs::create_dir_all(self.article_dir(id))?;
        atomic_write(&self.comments_path(id), page.html.as_bytes())?;
        store.set_discussion_comments(id, thread.count, now_secs(), page.html.len() as i64)?;
        Ok(thread.count)
    }

    /// Re-downloads the comments of a link post and rewrites the comments page.
    pub fn refresh_comments(&self, store: &mut Store, fetcher: &dyn Fetcher, id: ArticleId, ctl: &Control) -> Result<u32> {
        let rec = store.article_record(id)?.ok_or_else(|| Error::InvalidRequest("unknown article".into()))?;
        let d =
            discussion::detect(&rec.canonical_url, &rec.summary_html).ok_or_else(|| Error::InvalidRequest("not a discussion".into()))?;
        let article_url = if !rec.canonical_url.is_empty() && !discussion::is_discussion_url(&rec.canonical_url) {
            rec.canonical_url.as_str()
        } else {
            ""
        };
        store.insert_discussion(id, d.site.as_str(), &d.id, &d.url(), article_url)?;
        let thread = self.fetch_thread(fetcher, &d, ctl)?;
        self.write_comments(store, id, &d, &rec.title, &thread)
    }

    /// A Hacker News or Lobsters item: the full article (or post text) plus a comments page.
    fn cache_link_post(
        &self,
        store: &mut Store,
        fetcher: &dyn Fetcher,
        id: ArticleId,
        rec: &crate::store::ArticleRecord,
        d: &Discussion,
        ctl: &Control,
    ) -> Result<DownloadState> {
        let article_url0 = if !rec.canonical_url.is_empty() && !discussion::is_discussion_url(&rec.canonical_url) {
            rec.canonical_url.as_str()
        } else {
            ""
        };
        store.insert_discussion(id, d.site.as_str(), &d.id, &d.url(), article_url0)?;
        let row = store.discussion(id)?.unwrap_or_default();

        let mut thread: Option<Thread> = None;
        if !self.comments_path(id).is_file() || row.article_html.is_none() {
            match self.fetch_thread(fetcher, d, ctl) {
                Ok(t) => {
                    self.write_comments(store, id, d, &rec.title, &t)?;
                    thread = Some(t);
                }
                Err(Error::Storage(m)) => return Err(Error::Storage(m)),
                Err(e) => eprintln!("discussion {}: {e}", d.url()),
            }
        }

        let (html, base, raw_hash) = if let Some(h) = row.article_html.clone() {
            let base = if row.article_url.is_empty() { d.url() } else { row.article_url.clone() };
            let hash = format!("post:{}", sha256_hex(h.as_bytes()));
            (h, base, hash)
        } else {
            let linked = thread
                .as_ref()
                .and_then(|t| t.article_url.clone())
                .or_else(|| (!article_url0.is_empty()).then(|| article_url0.to_string()));
            if let Some(u) = linked {
                let feed_chars = prepare_article(&rec.summary_html, None, &ArticleMeta::default(), &AssetMap::new()).text.chars().count();
                let outcome = match extract_original(fetcher, &u, feed_chars, &self.cfg.limits, &ctl.cancel) {
                    Ok(o) => o,
                    Err(Error::Storage(m)) => return Err(Error::Storage(m)),
                    Err(_) => Extraction::Failed("the network is unavailable".into()),
                };
                match outcome {
                    Extraction::Extracted { html, .. } => {
                        store.set_discussion_article(id, &html, &u)?;
                        let hash = format!("post:{}", sha256_hex(html.as_bytes()));
                        (html, u, hash)
                    }
                    Extraction::Failed(why) => {
                        let html = if feed_chars >= MIN_TEXT_CHARS {
                            rec.summary_html.clone()
                        } else {
                            let mut e = String::new();
                            crate::content::sanitize::esc(&u, &mut e);
                            format!(
                                "<p>The full article could not be downloaded ({why}).</p><p><a href=\"{e}\">{e}</a></p>{}",
                                rec.summary_html
                            )
                        };
                        let hash = format!("feed:{}", sha256_hex(html.as_bytes()));
                        (html, u, hash)
                    }
                }
            } else {
                // A text post: the post itself is the article.
                match thread.as_ref().and_then(|t| t.post_html.clone()) {
                    Some(h) => {
                        store.set_discussion_article(id, &h, "")?;
                        let hash = format!("post:{}", sha256_hex(h.as_bytes()));
                        (h, d.url(), hash)
                    }
                    None => (rec.summary_html.clone(), d.url(), sha256_hex(rec.summary_html.as_bytes())),
                }
            }
        };
        self.process(store, fetcher, id, rec, &html, Some(&base), &raw_hash, ctl)
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
            discussion: store
                .discussion(id)?
                .map(|row| format!("Comments on {}", Site::from_name(&row.site).map(|s| s.label()).unwrap_or("the discussion"))),
        };
        let mut map = self.asset_map(store, id)?;
        let prepared = prepare_article(html, base, &meta, &map);
        for a in &prepared.assets {
            if !map.contains_key(&a.name) {
                store.set_asset(id, &a.name, &a.url, "pending", 0)?;
                map.insert(a.name.clone(), AssetState::Pending);
            }
        }
        let all_done = |m: &AssetMap, p: &crate::content::PreparedArticle| p.assets.iter().all(|a| is_done(m.get(&a.name)));
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
                s if is_done(s) => {}
                Some(AssetState::Failed) => failed |= !math::is_math_url(&a.url),
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
        let formulas = AtomicUsize::new(0);
        let formulas_failed = AtomicUsize::new(0);
        let formula_ms = AtomicU64::new(0);
        std::thread::scope(|scope| {
            for _ in 0..self.cfg.concurrency.clamp(1, 4) {
                let tx = tx.clone();
                let queue = &queue;
                let assets_dir = &assets_dir;
                let limits = &limits;
                let (formulas, formulas_failed, formula_ms) = (&formulas, &formulas_failed, &formula_ms);
                scope.spawn(move || loop {
                    if ctl.cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let job = queue.lock().unwrap().pop_front();
                    let Some((name, url)) = job else { break };
                    let result = if let Some((mode, tex)) = math::parse_math_url(&url) {
                        let t = Instant::now();
                        let rendered = math::render(tex, mode);
                        formulas.fetch_add(1, Ordering::Relaxed);
                        formula_ms.fetch_add(t.elapsed().as_millis() as u64, Ordering::Relaxed);
                        match rendered {
                            Ok(m) => {
                                let file = format!("{name}.png");
                                let metrics = format!("{:.4} {:.4}\n", m.width_em, m.depth_em);
                                atomic_write(&assets_dir.join(&file), &m.png)
                                    .and_then(|_| atomic_write(&assets_dir.join(format!("{name}.math")), metrics.as_bytes()))
                                    .map(|_| (file, m.png.len() as u64))
                                    .map_err(|e| match e {
                                        Error::Storage(m) => DownloadError::Transient(m),
                                        e => DownloadError::Permanent(e.to_string()),
                                    })
                            }
                            Err(why) => {
                                formulas_failed.fetch_add(1, Ordering::Relaxed);
                                eprintln!("math: {why}");
                                Err(DownloadError::Permanent(why))
                            }
                        }
                    } else {
                        match url::Url::parse(&url) {
                            Err(_) => Err(DownloadError::Permanent("bad URL".into())),
                            Ok(u) => download_image(fetcher, &u, limits, &ctl.cancel, &*ctl.sleep).and_then(|n| {
                                let file = format!("{name}.{}", n.ext);
                                atomic_write(&assets_dir.join(&file), &n.bytes).map(|_| (file, n.bytes.len() as u64)).map_err(|e| match e {
                                    Error::Storage(m) => DownloadError::Transient(m),
                                    e => DownloadError::Permanent(e.to_string()),
                                })
                            }),
                        }
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
        let n = formulas.load(Ordering::Relaxed);
        if n > 0 {
            eprintln!(
                "math: {n} formulas typeset in {} ms, {} not understood",
                formula_ms.load(Ordering::Relaxed),
                formulas_failed.load(Ordering::Relaxed)
            );
        }
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

    // ---- link pages -------------------------------------------------------------------------

    /// Where the reading view of a linked page is kept.
    pub fn web_path(&self, url: &str) -> PathBuf {
        self.dir.join("web").join(format!("{}.html", &sha256_hex(url.as_bytes())[..16]))
    }

    /// Fetches a linked page (https only), extracts its main text and writes it as a document
    /// without images. The inner result is the file, or a user-facing reason it cannot be shown.
    pub fn web_page(&self, fetcher: &dyn Fetcher, url: &str, ctl: &Control) -> Result<std::result::Result<PathBuf, String>> {
        let Ok(u) = url::Url::parse(url) else { return Ok(Err("this is not a web address".into())) };
        if u.scheme() != "https" {
            return Ok(Err("only https pages can be opened here".into()));
        }
        let fetched = match fetch_following(fetcher, &u, &self.cfg.limits, &ctl.cancel) {
            Ok(f) => f,
            Err(Error::InvalidRequest(m)) => return Ok(Err(format!("the page could not be loaded ({m})"))),
            Err(e) => return Err(e),
        };
        if !fetched.content_type.as_deref().map(|c| c.to_ascii_lowercase().contains("html")).unwrap_or(false) {
            return Ok(Err("the page is not HTML".into()));
        }
        let page = String::from_utf8_lossy(&fetched.body).into_owned();
        let Some(main) = extract_main(&page) else { return Ok(Err("no article text was found on the page".into())) };
        let (frag, text) = sanitize_fragment(&main, Some(url));
        if text.chars().count() < 200 {
            return Ok(Err("the page has too little text to show (it may need a login)".into()));
        }
        let host = u.host_str().unwrap_or("").to_string();
        let title = page_title(&page).unwrap_or_else(|| host.clone());
        let html = document(&title, &host, &frag, crate::content::typography::detect_language(&text));
        let path = self.web_path(url);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
            // Reading views are only kept for a week.
            if let Ok(entries) = fs::read_dir(dir) {
                for e in entries.flatten() {
                    let old = e
                        .metadata()
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age > Duration::from_secs(7 * 86_400));
                    if old {
                        let _ = fs::remove_file(e.path());
                    }
                }
            }
        }
        atomic_write(&path, html.as_bytes())?;
        Ok(Ok(path))
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

fn is_done(s: Option<&AssetState>) -> bool {
    matches!(s, Some(AssetState::Done(_) | AssetState::Formula { .. }))
}
