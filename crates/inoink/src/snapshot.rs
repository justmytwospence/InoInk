//! Bounded, read-only Inoreader snapshots for an offline desktop sandbox.
//!
//! OAuth rotations go only to the caller's token store. The output contains no credentials,
//! and is marked as a sandbox only after every imported article has verified cached content.

use crate::api::{ApiClient, AppConfig, Body, Clock, HttpRequest, HttpResponse, Method, Priority, TokenStore, Tokens, Transport};
use crate::cache::{atomic_write, Cache, CacheConfig};
use crate::content::images::Fetcher;
use crate::model::{ArticleQuery, Collection, DownloadState};
use crate::store::Store;
use crate::sync::{SyncConfig, SyncEngine};
use crate::{Error, Result};
use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::path::Path;
use std::sync::Arc;
use url::Url;

pub const SANDBOX_MARKER: &str = "preview-sandbox.json";
const PER_STREAM_LIMIT: usize = 100;
const MAX_ARTICLES: usize = 2 * PER_STREAM_LIMIT;

/// Marker contents need not be valid. Even a dangling symlink is an offline-only marker;
/// inability to inspect the marker also fails closed.
pub fn is_sandbox(data: &Path) -> bool {
    !matches!(fs::symlink_metadata(data.join(SANDBOX_MARKER)), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

/// Accept only the dedicated read-only OAuth scope, never a missing or expanded scope.
pub fn require_read_scope(tokens: &Tokens) -> Result<()> {
    let mut words = tokens.scope.split_whitespace().peekable();
    if words.peek().is_some() && words.all(|word| word == "read") {
        Ok(())
    } else {
        Err(Error::PermissionDenied("Snapshot authorization requires only the read scope.".into()))
    }
}

/// Validates both initial credentials and every rotation before persistence or use.
pub struct ReadOnlyTokens {
    inner: Arc<dyn TokenStore>,
}

impl ReadOnlyTokens {
    pub fn new(inner: Arc<dyn TokenStore>) -> Self {
        Self { inner }
    }
}

impl TokenStore for ReadOnlyTokens {
    fn load(&self) -> Result<Option<Tokens>> {
        let tokens = self.inner.load()?;
        if let Some(tokens) = &tokens {
            require_read_scope(tokens)?;
        }
        Ok(tokens)
    }

    fn save(&self, tokens: &Tokens) -> Result<()> {
        require_read_scope(tokens)?;
        self.inner.save(tokens)
    }
}

fn denied() -> Error {
    Error::PermissionDenied("Request is not permitted by the snapshot read-only policy.".into())
}

fn parse_url(raw: &str) -> Result<Url> {
    // Reject ambiguities before URL normalization (notably dot segments and backslashes).
    // Encoded slashes remain valid in stream IDs, but cannot conceal traversal segments.
    if raw.chars().any(|c| c.is_control() || c.is_whitespace()) || raw.contains('\\') {
        return Err(denied());
    }
    let url = Url::parse(raw).map_err(|_| denied())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(denied());
    }
    let authority_and_path = raw.split_once("://").ok_or_else(denied)?.1.split(['?', '#']).next().unwrap_or("");
    // Empty userinfo (`https://@host/`) is also forbidden.
    let (authority, path) = authority_and_path.split_once('/').unwrap_or((authority_and_path, ""));
    if authority.is_empty() || authority.contains('@') {
        return Err(denied());
    }
    // Also reject nested encodings of traversal; never depend on how many times a server
    // decodes a stream path. Decoding strictly shortens the path until it stops changing.
    let mut path = path.to_string();
    loop {
        let decoded = percent_encoding::percent_decode_str(&path).decode_utf8().map_err(|_| denied())?;
        if decoded.contains('\\') || decoded.chars().any(char::is_control) || decoded.split('/').any(|part| matches!(part, "." | "..")) {
            return Err(denied());
        }
        if decoded == path {
            break;
        }
        path = decoded.into_owned();
    }
    Ok(url)
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme() && a.host() == b.host() && a.port_or_known_default() == b.port_or_known_default()
}

/// Exact read-endpoint allowlist, not a blanket GET policy: Inoreader also has destructive
/// GETs. The injected production transport must not follow redirects (UreqTransport does not).
pub struct ReadOnlyTransport {
    inner: Arc<dyn Transport>,
    base: Url,
    token: Url,
}

impl ReadOnlyTransport {
    /// Invalid endpoint configuration is rejected without exposing URLs or credentials.
    pub fn new(inner: Arc<dyn Transport>, cfg: &AppConfig) -> Result<Self> {
        let base = parse_url(&cfg.base_url)?;
        let token = parse_url(&cfg.token_url)?;
        if base.query().is_some() || token.query().is_some() {
            return Err(denied());
        }
        Ok(Self { inner, base, token })
    }

    fn read_path<'a>(&self, url: &'a Url) -> Option<&'a str> {
        if !same_origin(url, &self.base) {
            return None;
        }
        let path = url.path().strip_prefix(self.base.path().trim_end_matches('/'))?;
        match path {
            "/user-info" | "/subscription/list" | "/tag/list" | "/unread-count" | "/preference/stream/list" => Some(path),
            p if p.strip_prefix("/stream/contents/").is_some_and(|stream| !stream.is_empty()) => Some(p),
            _ => None,
        }
    }
}

impl Transport for ReadOnlyTransport {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse> {
        let url = parse_url(&req.url)?;
        let read = req.method == Method::Get && req.body.is_none() && self.read_path(&url).is_some();
        let token = req.method == Method::Post
            && same_origin(&url, &self.token)
            && url.path() == self.token.path()
            && url.query().is_none()
            && matches!(&req.body, Some(Body::Form(form)) if {
                let mut grants = form.iter().filter(|(key, _)| key == "grant_type");
                matches!(grants.next(), Some((_, value)) if matches!(value.as_str(), "authorization_code" | "refresh_token"))
                    && grants.next().is_none()
            });
        if !read && !token {
            return Err(denied());
        }
        let response = self.inner.send(req)?;
        if response.body.len() > crate::api::MAX_BODY {
            return Err(Error::Data("Snapshot response is too large.".into()));
        }
        // Fail rather than ingest an oversized page from a server ignoring the requested n=100.
        if read && self.read_path(&url).is_some_and(|p| p.starts_with("/stream/contents/")) && (200..300).contains(&response.status) {
            let value: serde_json::Value =
                serde_json::from_slice(&response.body).map_err(|_| Error::Data("Invalid snapshot page.".into()))?;
            if value["items"].as_array().is_some_and(|items| items.len() > PER_STREAM_LIMIT) {
                return Err(Error::Data("Snapshot page exceeds the article limit.".into()));
            }
        }
        Ok(response)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotReport {
    pub articles: usize,
    pub incomplete_images: usize,
}

/// Downloads one page each of newest unread and starred articles, then caches and verifies
/// every unique article under account `local`. Never starts a worker or a sync cycle.
///
/// `output` must not exist and its parent must exist. On failure an unmarked, private partial
/// directory may remain; callers may remove only the fresh output owned by their invocation.
/// The caller must supply a dedicated external token store and an unauthenticated Fetcher.
pub fn download(
    output: &Path,
    cfg: AppConfig,
    transport: Arc<dyn Transport>,
    clock: Arc<dyn Clock>,
    tokens: Arc<dyn TokenStore>,
    fetcher: Arc<dyn Fetcher>,
) -> Result<SnapshotReport> {
    match fs::symlink_metadata(output) {
        Ok(_) => return Err(Error::InvalidRequest("Snapshot output already exists.".into())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let transport = Arc::new(ReadOnlyTransport::new(transport, &cfg)?);
    let tokens = Arc::new(ReadOnlyTokens::new(tokens));
    let api = ApiClient::new(cfg, transport, clock.clone(), tokens)?.with_priority(Priority::Explicit);
    fs::DirBuilder::new().mode(0o700).create(output)?;
    let report = {
        api.user_info()?;
        let store = Store::open(&output.join("inoink.db"), "local")?;
        let cache_cfg = CacheConfig { extract_stubs: true, ..Default::default() };
        let cache = Cache::new(&output.join("cache"), "local", cache_cfg.clone()).with_diagnostics(false);
        let mut engine = SyncEngine::new(store, api, cache, fetcher, clock.clone(), SyncConfig::default());
        engine.refresh_organization(Priority::Explicit)?;
        // Deliberately discard both continuation tokens; a snapshot is not an account export.
        engine.browse(&Collection::Unread, None)?;
        engine.browse(&Collection::Starred, None)?;
        let query = ArticleQuery {
            collection: None,
            unread_only: false,
            search: None,
            limit: MAX_ARTICLES,
            group: Default::default(),
            sort: Default::default(),
            after: None,
            visit_start_usec: 0,
        };
        let ids = engine.store.query_ids(&query, MAX_ARTICLES)?;
        if engine.store.count_articles(&query)? != ids.len() {
            return Err(Error::Data("Snapshot exceeds the article limit.".into()));
        }
        let mut incomplete_images = 0;
        for &id in &ids {
            let state = engine.cache_now(id)?;
            if !state.readable_offline()
                || !engine.store.download_state(id)?.readable_offline()
                || engine.store.latest_version(id)?.is_none()
            {
                return Err(Error::Data("Snapshot contains unreadable content.".into()));
            }
            incomplete_images += usize::from(matches!(state, DownloadState::TextReady | DownloadState::MissingImages));
        }
        let cache = Cache::new(&output.join("cache"), "local", cache_cfg).with_diagnostics(false);
        let verification = cache.verify(&engine.store)?;
        if !verification.problems.is_empty() || verification.checked != ids.len() {
            return Err(Error::Data("Snapshot cache verification failed.".into()));
        }
        engine.store.set_meta("preview_source", "inoreader")?;
        engine.store.checkpoint()?;
        SnapshotReport { articles: ids.len(), incomplete_images }
    }; // Drop all database/cache/API handles before publishing the marker.
    let marker = serde_json::json!({
        "format": 1,
        "source": "inoreader",
        "downloaded_at": clock.now(),
        "articles": report.articles,
        "incomplete_images": report.incomplete_images,
        "per_stream_limit": PER_STREAM_LIMIT,
    });
    atomic_write(&output.join(SANDBOX_MARKER), &serde_json::to_vec(&marker)?)?;
    Ok(report)
}
