//! Inoreader HTTP client. Transport, clock and token storage are injected so every request
//! shape and failure mode is testable without a network.

pub mod auth;
pub mod quota;
pub mod types;

use crate::error::{Error, Result};
use crate::model::*;
pub use auth::{AppConfig, TokenStore, Tokens};
pub use quota::{Priority, Quota, Zone};
use serde_json::Value;
use std::sync::{Arc, Mutex};
pub use types::*;
use url::form_urlencoded;

/// Largest response body accepted from the API.
pub const MAX_BODY: usize = 16 * 1024 * 1024;
/// Largest number of articles in one edit-tag request.
pub const MAX_EDIT_IDS: usize = 250;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Form(Vec<(String, String)>),
    Json(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Body>,
}

#[derive(Clone, Debug, Default)]
pub struct HttpResponse {
    pub status: u16,
    /// Header names lower-cased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// Sends one HTTP request. `Err(Offline)` means nothing reached the server;
/// `Err(UnknownOutcome)` means the request may have been processed.
pub trait Transport: Send + Sync {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse>;
}

pub trait Clock: Send + Sync {
    fn now(&self) -> i64;
}

pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> i64 {
        crate::util::now_secs()
    }
}

struct Inner {
    cfg: AppConfig,
    transport: Arc<dyn Transport>,
    clock: Arc<dyn Clock>,
    store: Arc<dyn TokenStore>,
    tokens: Mutex<Option<Tokens>>,
    quota: Mutex<Quota>,
}

#[derive(Clone)]
pub struct ApiClient {
    inner: Arc<Inner>,
    prio: Priority,
}

fn encode_component(s: &str) -> String {
    // Everything except unreserved characters, matching the documented `%2F` examples.
    const SET: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');
    percent_encoding::utf8_percent_encode(s, SET).to_string()
}

fn expect_ok(resp: &HttpResponse) -> Result<()> {
    let body = String::from_utf8_lossy(&resp.body);
    if body.trim().eq_ignore_ascii_case("ok") {
        Ok(())
    } else {
        Err(Error::InvalidRequest("server did not acknowledge the change".into()))
    }
}

impl ApiClient {
    pub fn new(cfg: AppConfig, transport: Arc<dyn Transport>, clock: Arc<dyn Clock>, store: Arc<dyn TokenStore>) -> Result<ApiClient> {
        let tokens = store.load()?;
        Ok(ApiClient {
            inner: Arc::new(Inner { cfg, transport, clock, store, tokens: Mutex::new(tokens), quota: Mutex::new(Quota::default()) }),
            prio: Priority::Background,
        })
    }

    /// A handle sharing tokens and quota but using `prio` for its requests.
    pub fn with_priority(&self, prio: Priority) -> ApiClient {
        ApiClient { inner: self.inner.clone(), prio }
    }

    pub fn restore_quota(&self, json: &str) {
        *self.inner.quota.lock().unwrap() = Quota::from_json(json);
    }

    pub fn quota_json(&self) -> String {
        self.inner.quota.lock().unwrap().to_json()
    }

    pub fn quota(&self) -> Quota {
        self.inner.quota.lock().unwrap().clone()
    }

    pub fn has_tokens(&self) -> bool {
        self.inner.tokens.lock().unwrap().is_some()
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.inner.cfg.base_url, path)
    }

    fn base_headers(&self, access: &str) -> Vec<(String, String)> {
        let mut h =
            vec![("Authorization".to_string(), format!("Bearer {access}")), ("User-Agent".to_string(), self.inner.cfg.user_agent.clone())];
        if let (Some(id), Some(key)) = (&self.inner.cfg.app_id, &self.inner.cfg.app_key) {
            h.push(("AppId".into(), id.clone()));
            h.push(("AppKey".into(), key.clone()));
        }
        h
    }

    /// Returns a usable access token, refreshing it (single flight) when expired or `force`.
    fn access_token(&self, force: bool) -> Result<String> {
        let mut guard = self.inner.tokens.lock().unwrap();
        let now = self.inner.clock.now();
        let cur = guard.clone().ok_or(Error::NeedsAuthorization)?;
        if !force && cur.expires_at > now + 60 {
            return Ok(cur.access_token);
        }
        let new = auth::refresh(&self.inner.cfg, self.inner.transport.as_ref(), self.inner.clock.as_ref(), &cur.refresh_token)?;
        // Persist the (possibly rotated) refresh token before anything uses it.
        self.inner.store.save(&new)?;
        let access = new.access_token.clone();
        *guard = Some(new);
        Ok(access)
    }

    fn call(&self, zone: Zone, method: Method, path: &str, body: Option<Body>) -> Result<HttpResponse> {
        let now = self.inner.clock.now();
        {
            let mut q = self.inner.quota.lock().unwrap();
            q.tick(now);
            if let Some(wait) = q.blocked_for(zone, self.prio, now) {
                return Err(Error::QuotaPaused { reset_after_secs: wait });
            }
        }
        let mut refreshed = false;
        loop {
            let access = self.access_token(false)?;
            let req = HttpRequest { method, url: self.url(path), headers: self.base_headers(&access), body: body.clone() };
            self.inner.quota.lock().unwrap().note_sent(zone);
            let resp = match self.inner.transport.send(&req) {
                Ok(r) => r,
                Err(Error::UnknownOutcome(m)) if method == Method::Get => return Err(Error::Retryable(m)),
                Err(e) => return Err(e),
            };
            let now = self.inner.clock.now();
            if resp.body.len() > MAX_BODY {
                return Err(Error::Data("response too large".into()));
            }
            {
                let mut q = self.inner.quota.lock().unwrap();
                q.observe(&resp.headers, now);
            }
            match resp.status {
                200..=299 => return Ok(resp),
                401 if !refreshed => {
                    refreshed = true;
                    self.access_token(true)?;
                    continue;
                }
                401 => return Err(Error::NeedsAuthorization),
                429 => {
                    let retry = resp.header("retry-after").and_then(|v| v.trim().parse::<u64>().ok());
                    let reset = resp.header("x-reader-limits-reset-after").and_then(|v| v.trim().parse::<u64>().ok());
                    let mut q = self.inner.quota.lock().unwrap();
                    q.pause(retry.or(reset), now);
                    let until = q.paused_until().unwrap_or(now);
                    return Err(Error::QuotaPaused { reset_after_secs: (until - now).max(1) as u64 });
                }
                403 => return Err(Error::PermissionDenied("not permitted or not implemented".into())),
                400 | 404 | 405 | 422 => return Err(Error::InvalidRequest(format!("request rejected ({})", resp.status))),
                s if s >= 500 => return Err(Error::Retryable(format!("server error {s}"))),
                s => return Err(Error::Retryable(format!("unexpected status {s}"))),
            }
        }
    }

    fn json(&self, zone: Zone, path: &str) -> Result<Value> {
        let resp = self.call(zone, Method::Get, path, None)?;
        serde_json::from_slice(&resp.body).map_err(|_| Error::Data("response is not valid JSON".into()))
    }

    fn post_form(&self, zone: Zone, path: &str, form: Vec<(String, String)>) -> Result<HttpResponse> {
        self.call(zone, Method::Post, path, Some(Body::Form(form)))
    }

    // ---- reads (zone 1) -------------------------------------------------------------------

    pub fn user_info(&self) -> Result<UserInfo> {
        parse_user_info(&self.json(Zone::One, "/user-info")?)
    }

    pub fn subscriptions(&self) -> Result<Vec<crate::store::Subscription>> {
        parse_subscriptions(&self.json(Zone::One, "/subscription/list")?)
    }

    pub fn tags(&self) -> Result<Vec<crate::store::TagInfo>> {
        parse_tags(&self.json(Zone::One, "/tag/list?types=1&counts=1")?)
    }

    pub fn unread_counts(&self) -> Result<UnreadCounts> {
        parse_unread_counts(&self.json(Zone::One, "/unread-count?output=json")?)
    }

    pub fn contents(&self, stream: &StreamId, p: &ContentsParams) -> Result<StreamPage> {
        let mut q = form_urlencoded::Serializer::new(String::new());
        q.append_pair("n", &p.count.clamp(1, 100).to_string());
        if p.oldest_first {
            q.append_pair("r", "o");
        }
        if let Some(ot) = p.newer_than {
            q.append_pair("ot", &ot.to_string());
        }
        if p.exclude_read {
            q.append_pair("xt", STATE_READ);
        }
        if p.only_starred {
            q.append_pair("it", STATE_STARRED);
        } else if p.only_liked {
            q.append_pair("it", STATE_LIKE);
        }
        if let Some(c) = &p.continuation {
            q.append_pair("c", c);
        }
        if p.annotations {
            q.append_pair("annotations", "1");
        }
        if p.summaries {
            q.append_pair("summaries", "1");
        }
        if p.manual_tags_only {
            q.append_pair("includeAllDirectStreamIds", "false");
        }
        let path = format!("/stream/contents/{}?{}", encode_component(stream.as_str()), q.finish());
        parse_stream_page(&self.json(Zone::One, &path)?)
    }

    pub fn item_ids(&self, p: &IdsParams) -> Result<IdPage> {
        let mut q = form_urlencoded::Serializer::new(String::new());
        q.append_pair("s", p.stream.as_str());
        q.append_pair("n", &p.count.clamp(1, 1000).to_string());
        q.append_pair("output", "json");
        if p.oldest_first {
            q.append_pair("r", "o");
        }
        if let Some(ot) = p.newer_than {
            q.append_pair("ot", &ot.to_string());
        }
        if p.exclude_read {
            q.append_pair("xt", STATE_READ);
        }
        if p.only_starred {
            q.append_pair("it", STATE_STARRED);
        } else if p.only_liked {
            q.append_pair("it", STATE_LIKE);
        }
        if let Some(c) = &p.continuation {
            q.append_pair("c", c);
        }
        if !p.direct_ids {
            // The documented value is the string `false`, not `0`.
            q.append_pair("includeAllDirectStreamIds", "false");
        }
        parse_id_page(&self.json(Zone::One, &format!("/stream/items/ids?{}", q.finish()))?)
    }

    pub fn stream_preferences(&self) -> Result<std::collections::BTreeMap<String, String>> {
        parse_stream_prefs(&self.json(Zone::One, "/preference/stream/list")?)
    }

    // ---- writes (zone 2) ------------------------------------------------------------------

    /// Sets and/or clears tags on up to [`MAX_EDIT_IDS`] articles. A state setter: safe to repeat.
    pub fn edit_tag(&self, ids: &[ArticleId], add: Option<&Membership>, remove: Option<&Membership>) -> Result<()> {
        if ids.is_empty() || (add.is_none() && remove.is_none()) {
            return Err(Error::InvalidRequest("nothing to change".into()));
        }
        if ids.len() > MAX_EDIT_IDS {
            return Err(Error::InvalidRequest(format!("more than {MAX_EDIT_IDS} articles")));
        }
        let mut form = Vec::new();
        if let Some(a) = add {
            form.push(("a".to_string(), a.tag_id()));
        }
        if let Some(r) = remove {
            form.push(("r".to_string(), r.tag_id()));
        }
        for id in ids {
            form.push(("i".to_string(), id.to_short()));
        }
        expect_ok(&self.post_form(Zone::Two, "/edit-tag", form)?)
    }

    /// Not idempotent: an unknown outcome must be reconciled against the subscription list.
    pub fn quickadd(&self, feed: &str) -> Result<QuickAdd> {
        let resp = self.post_form(Zone::Two, "/subscription/quickadd", vec![("quickadd".into(), feed.into())])?;
        parse_quickadd(&serde_json::from_slice(&resp.body).map_err(|_| Error::Data("quickadd response is not JSON".into()))?)
    }

    pub fn edit_subscription(&self, e: &SubscriptionEdit) -> Result<()> {
        let ac = match e.action {
            SubAction::Edit => "edit",
            SubAction::Subscribe => "subscribe",
            SubAction::Unsubscribe => "unsubscribe",
        };
        let mut form = vec![("ac".to_string(), ac.to_string()), ("s".to_string(), e.stream.0.clone())];
        if let Some(t) = &e.title {
            form.push(("t".into(), t.clone()));
        }
        if let Some(f) = &e.add_folder {
            form.push(("a".into(), StreamId::label(f).0));
        }
        if let Some(f) = &e.remove_folder {
            form.push(("r".into(), StreamId::label(f).0));
        }
        expect_ok(&self.post_form(Zone::Two, "/subscription/edit", form)?)
    }

    pub fn rename_tag(&self, source: &StreamId, new_name: &str) -> Result<()> {
        expect_ok(&self.post_form(
            Zone::Two,
            "/rename-tag",
            vec![("s".into(), source.0.clone()), ("dest".into(), StreamId::label(new_name).0)],
        )?)
    }

    pub fn disable_tag(&self, stream: &StreamId) -> Result<()> {
        expect_ok(&self.post_form(Zone::Two, "/disable-tag", vec![("s".into(), stream.0.clone())])?)
    }

    /// `sort_ids` are eight-hex-digit ids in the desired order.
    pub fn set_stream_order(&self, stream: &StreamId, sort_ids: &[String]) -> Result<()> {
        let value = validate_sort_ids(sort_ids)?;
        expect_ok(&self.post_form(
            Zone::Two,
            "/preference/stream/set",
            vec![("s".into(), stream.0.clone()), ("k".into(), "subscription-ordering".into()), ("v".into(), value)],
        )?)
    }

    /// Marks a whole stream read up to `displayed_ts` (unix seconds or microseconds).
    /// Not used for UI bulk actions: a time cutoff cannot guarantee scope.
    pub fn mark_all_read(&self, stream: &StreamId, displayed_ts: i64) -> Result<()> {
        expect_ok(&self.post_form(
            Zone::Two,
            "/mark-all-as-read",
            vec![("s".into(), stream.0.clone()), ("ts".into(), displayed_ts.to_string())],
        )?)
    }

    pub fn create_active_search(&self, spec: &ActiveSearchSpec) -> Result<ActiveSearchCreated> {
        if spec.term.trim().is_empty() {
            return Err(Error::InvalidRequest("search term is empty".into()));
        }
        let mut o = serde_json::Map::new();
        o.insert("term".into(), spec.term.clone().into());
        o.insert(
            "filter_type".into(),
            match spec.filter_type {
                FilterType::Folder => "folder",
                FilterType::Subscription => "subscription",
                FilterType::Public => "public",
            }
            .into(),
        );
        match (&spec.filter_type, &spec.filter_id) {
            (FilterType::Public, _) => {}
            (_, Some(id)) => {
                o.insert("filter_id".into(), id.clone().into());
            }
            (_, None) => return Err(Error::InvalidRequest("filter_id is required unless public".into())),
        }
        if !spec.article_ids.is_empty() {
            o.insert("article_ids".into(), spec.article_ids.iter().map(|a| Value::from(a.to_short())).collect::<Vec<_>>().into());
        }
        for (k, v) in [
            ("search_filters", &spec.search_filters),
            ("search_match", &spec.search_match),
            ("search_feed_popularity", &spec.search_feed_popularity),
        ] {
            if let Some(v) = v {
                o.insert(k.into(), v.clone().into());
            }
        }
        let resp = self.call(Zone::Two, Method::Post, "/active_search/create", Some(Body::Json(Value::Object(o).to_string())))?;
        let v: Value = serde_json::from_slice(&resp.body).map_err(|_| Error::Data("active search response is not JSON".into()))?;
        // Validation failures can arrive as HTTP 202 with an error description.
        if resp.status == 202
            || v.get("accountLimit").is_some()
            || v.get("needUpgrade").is_some()
            || v.get("userOutput").is_some()
            || v.get("error").is_some()
        {
            let why = v["userOutput"].as_str().or(v["error"].as_str()).unwrap_or("the search was not created");
            return Err(Error::InvalidRequest(why.to_string()));
        }
        let id = v["id"].as_str().ok_or_else(|| Error::Data("active search response has no id".into()))?;
        Ok(ActiveSearchCreated { id: normalize_user(id), title: v["title"].as_str().unwrap_or("").to_string() })
    }

    /// Documented as a GET that deletes. Never retried automatically by callers.
    pub fn delete_active_search(&self, id: &StreamId) -> Result<()> {
        let q = form_urlencoded::Serializer::new(String::new()).append_pair("id", id.as_str()).finish();
        let resp = self.call(Zone::Two, Method::Get, &format!("/active_search/delete?{q}"), None)?;
        expect_ok(&resp)
    }
}

/// Real HTTPS transport over `ureq` with bounded sizes and timeouts.
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    pub fn new() -> UreqTransport {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(std::time::Duration::from_secs(10)))
            .timeout_global(Some(std::time::Duration::from_secs(60)))
            .max_redirects(0)
            .build()
            .new_agent();
        UreqTransport { agent }
    }
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self::new()
    }
}

fn map_ureq(e: ureq::Error, sent_body: bool) -> Error {
    use ureq::Error as E;
    if matches!(e, E::HostNotFound | E::ConnectionFailed | E::Io(_) | E::Timeout(_)) {
        eprintln!("inoink: network error: {e}");
    }
    match e {
        E::HostNotFound | E::ConnectionFailed | E::Io(_) if !sent_body => Error::Offline,
        E::HostNotFound | E::ConnectionFailed => Error::Offline,
        E::Timeout(_) | E::Io(_) => Error::UnknownOutcome("connection lost while waiting for the response".into()),
        other => Error::Retryable(other.to_string().chars().take(80).collect()),
    }
}

impl Transport for UreqTransport {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse> {
        let mut resp = match (req.method, &req.body) {
            (Method::Get, _) => {
                let mut r = self.agent.get(&req.url);
                for (k, v) in &req.headers {
                    r = r.header(k, v);
                }
                r.call().map_err(|e| map_ureq(e, false))?
            }
            (Method::Post, body) => {
                let mut r = self.agent.post(&req.url);
                for (k, v) in &req.headers {
                    r = r.header(k, v);
                }
                match body {
                    Some(Body::Form(pairs)) => {
                        let encoded = form_urlencoded::Serializer::new(String::new()).extend_pairs(pairs).finish();
                        r.header("Content-Type", "application/x-www-form-urlencoded").send(encoded).map_err(|e| map_ureq(e, true))?
                    }
                    Some(Body::Json(j)) => r.header("Content-Type", "application/json").send(j).map_err(|e| map_ureq(e, true))?,
                    None => r.send_empty().map_err(|e| map_ureq(e, true))?,
                }
            }
        };
        let status = resp.status().as_u16();
        let headers = resp.headers().iter().map(|(k, v)| (k.as_str().to_ascii_lowercase(), v.to_str().unwrap_or("").to_string())).collect();
        let body = resp.body_mut().with_config().limit(MAX_BODY as u64 + 1).read_to_vec().map_err(|e| map_ureq(e, true))?;
        Ok(HttpResponse { status, headers, body })
    }
}
