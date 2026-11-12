#![allow(dead_code, clippy::redundant_closure_call, clippy::type_complexity)]
//! A small in-memory Inoreader: just enough of the documented API to exercise sync, with
//! fault injection (offline, lost responses, status codes, quota, hooks).

use inoink::api::*;
use inoink::error::{Error, Result};
use inoink::model::ArticleId;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct SArt {
    pub id: u64,
    pub ts: i64,
    pub title: String,
    pub html: String,
    pub feed: String,
    pub read: bool,
    pub starred: bool,
    pub like: bool,
    pub labels: BTreeSet<String>,
    pub annotations: Vec<serde_json::Value>,
}

#[derive(Clone, Debug)]
pub struct SSub {
    pub id: String,
    pub title: String,
    pub url: String,
    pub folders: Vec<String>,
    pub first_usec: i64,
}

pub enum Kind {
    Offline,
    /// Apply the request on the server, then lose the response.
    LostResponse,
    Status(u16),
    Hook(Box<dyn FnMut(&mut State) + Send>),
    /// Return a continuation token that never changes.
    LoopToken,
}

pub struct Fault {
    pub path: String,
    pub kind: Kind,
    pub times: usize,
    /// Skip this many matching requests before the fault starts.
    pub after: usize,
}

pub struct State {
    pub arts: BTreeMap<u64, SArt>,
    pub subs: Vec<SSub>,
    pub tags: Vec<(String, String)>,
    pub page_cap: usize,
    pub faults: Vec<Fault>,
    pub log: Vec<String>,
    pub valid_access: String,
    pub valid_refresh: String,
    pub next_token: u32,
    pub usage: [u32; 2],
    pub limits: [u32; 2],
    pub reset_after: u32,
    pub headers_on: bool,
}

pub struct Fake {
    pub st: Mutex<State>,
}

fn zone_of(path: &str) -> usize {
    if [
        "/edit-tag",
        "/mark-all-as-read",
        "/subscription/quickadd",
        "/subscription/edit",
        "/rename-tag",
        "/disable-tag",
        "/preference/stream/set",
        "/active_search",
    ]
    .iter()
    .any(|p| path.starts_with(p))
    {
        1
    } else {
        0
    }
}

impl Fake {
    pub fn new() -> Arc<Fake> {
        Arc::new(Fake {
            st: Mutex::new(State {
                arts: BTreeMap::new(),
                subs: vec![],
                tags: vec![],
                page_cap: 100,
                faults: vec![],
                log: vec![],
                valid_access: "acc".into(),
                valid_refresh: "ref".into(),
                next_token: 1,
                usage: [0, 0],
                limits: [1000, 1000],
                reset_after: 3600,
                headers_on: true,
            }),
        })
    }

    pub fn art(&self, id: u64, ts: i64, feed: &str) {
        let mut s = self.st.lock().unwrap();
        s.arts.insert(
            id,
            SArt {
                id,
                ts,
                title: format!("Article {id}"),
                html: format!("<p>Body of article {id}</p>"),
                feed: feed.into(),
                read: false,
                starred: false,
                like: false,
                labels: BTreeSet::new(),
                annotations: vec![],
            },
        );
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        f(&mut self.st.lock().unwrap())
    }

    pub fn fail(&self, path: &str, kind: Kind, times: usize) {
        self.st.lock().unwrap().faults.push(Fault { path: path.into(), kind, times, after: 0 });
    }

    pub fn fail_after(&self, path: &str, after: usize, kind: Kind, times: usize) {
        self.st.lock().unwrap().faults.push(Fault { path: path.into(), kind, times, after });
    }

    pub fn log(&self) -> Vec<String> {
        self.st.lock().unwrap().log.clone()
    }

    pub fn count(&self, needle: &str) -> usize {
        self.log().iter().filter(|l| l.contains(needle)).count()
    }

    pub fn clear_log(&self) {
        self.st.lock().unwrap().log.clear();
    }

    pub fn a(&self, id: u64) -> SArt {
        self.st.lock().unwrap().arts[&id].clone()
    }
}

fn items_json(arts: &[SArt], subs: &[SSub], with_annotations: bool) -> serde_json::Value {
    let items: Vec<_> = arts
        .iter()
        .map(|a| {
            let mut cats = vec!["user/42/state/com.google/reading-list".to_string()];
            if a.read {
                cats.push("user/42/state/com.google/read".into());
            }
            if a.starred {
                cats.push("user/42/state/com.google/starred".into());
            }
            if a.like {
                cats.push("user/42/state/com.google/like".into());
            }
            for l in &a.labels {
                cats.push(format!("user/42/label/{l}"));
            }
            // Folder tags are inherited from the feed's folders, as the real API does.
            if let Some(s) = subs.iter().find(|s| s.id == a.feed) {
                for f in &s.folders {
                    cats.push(format!("user/42/label/{f}"));
                }
            }
            let mut item = serde_json::json!({
                "id": ArticleId(a.id).to_long(),
                "timestampUsec": a.ts.to_string(),
                "crawlTimeMsec": (a.ts / 1000).to_string(),
                "categories": cats,
                "title": a.title,
                "published": a.ts / 1_000_000,
                "canonical": [{"href": format!("https://blog.example/post/{}", a.id)}],
                "summary": {"direction": "ltr", "content": a.html},
                "author": "Au",
                "origin": {"streamId": a.feed, "title": "Blog", "htmlUrl": "https://blog.example/"}
            });
            if with_annotations {
                item["annotations"] = serde_json::Value::Array(a.annotations.clone());
            }
            item
        })
        .collect();
    serde_json::Value::Array(items)
}

impl Transport for Fake {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse> {
        let url = url::Url::parse(&req.url).unwrap();
        let path = url.path().to_string();
        let path = path.strip_prefix("/reader/api/0").unwrap_or(&path).to_string();
        let mut st = self.st.lock().unwrap();
        let q: Vec<(String, String)> = url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
        let body_form: Vec<(String, String)> = match &req.body {
            Some(Body::Form(f)) => f.clone(),
            _ => vec![],
        };
        st.log.push(format!(
            "{:?} {}{}",
            req.method,
            path,
            if url.query().is_some() { format!("?{}", url.query().unwrap()) } else { String::new() }
        ));
        if !body_form.is_empty() {
            let line = body_form
                .iter()
                .filter(|(k, _)| k != "client_secret" && k != "refresh_token")
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("&");
            st.log.push(format!("  body {line}"));
        }

        // Fault injection.
        let mut lost = false;
        let mut idx = None;
        for (i, f) in st.faults.iter().enumerate() {
            if f.times > 0 && path.contains(&f.path) {
                idx = Some(i);
                break;
            }
        }
        let mut loop_token = false;
        if let Some(i) = idx {
            if st.faults[i].after > 0 {
                st.faults[i].after -= 1;
            } else {
                st.faults[i].times -= 1;
                match &mut st.faults[i].kind {
                    Kind::Offline => return Err(Error::Offline),
                    Kind::Status(s) => return Ok(HttpResponse { status: *s, headers: vec![], body: b"".to_vec() }),
                    Kind::LostResponse => lost = true,
                    Kind::LoopToken => loop_token = true,
                    Kind::Hook(_) => {
                        let mut k = std::mem::replace(&mut st.faults[i].kind, Kind::Offline);
                        if let Kind::Hook(h) = &mut k {
                            h(&mut st);
                        }
                        st.faults[i].kind = k;
                        st.faults[i].times = 0;
                    }
                }
            }
        }

        // Token endpoint.
        if req.url.contains("/oauth2/token") {
            let get = |k: &str| body_form.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone()).unwrap_or_default();
            if get("refresh_token") != st.valid_refresh {
                return Ok(HttpResponse { status: 400, headers: vec![], body: br#"{"error":"invalid_grant"}"#.to_vec() });
            }
            st.next_token += 1;
            st.valid_access = format!("acc{}", st.next_token);
            st.valid_refresh = format!("ref{}", st.next_token);
            let b = serde_json::json!({"access_token": st.valid_access, "refresh_token": st.valid_refresh, "expires_in": 3600, "scope": "read write"});
            return Ok(HttpResponse { status: 200, headers: vec![], body: b.to_string().into_bytes() });
        }

        let auth = req.headers.iter().find(|(k, _)| k == "Authorization").map(|(_, v)| v.clone()).unwrap_or_default();
        if auth != format!("Bearer {}", st.valid_access) {
            return Ok(HttpResponse { status: 401, headers: vec![], body: vec![] });
        }
        let z = zone_of(&path);
        let mk_headers = |st: &State| -> Vec<(String, String)> {
            if !st.headers_on {
                return vec![];
            }
            vec![
                ("x-reader-zone1-limit".into(), st.limits[0].to_string()),
                ("x-reader-zone2-limit".into(), st.limits[1].to_string()),
                ("x-reader-zone1-usage".into(), st.usage[0].to_string()),
                ("x-reader-zone2-usage".into(), st.usage[1].to_string()),
                ("x-reader-limits-reset-after".into(), st.reset_after.to_string()),
            ]
        };
        if st.usage[z] >= st.limits[z] {
            let h = mk_headers(&st);
            return Ok(HttpResponse { status: 429, headers: h, body: vec![] });
        }
        st.usage[z] += 1;
        let headers = mk_headers(&st);
        let ok = |body: String, h: Vec<(String, String)>| Ok(HttpResponse { status: 200, headers: h, body: body.into_bytes() });
        let qget = |k: &str| q.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone());
        let ids_of = |form: &[(String, String)]| -> Vec<u64> {
            form.iter().filter(|(k, _)| k == "i").map(|(_, v)| ArticleId::parse(v).unwrap().0).collect()
        };

        let result: Result<HttpResponse> = (|| {
            match path.as_str() {
                "/user-info" => ok(r#"{"userId":"42","userName":"T","userEmail":"t@example.org"}"#.into(), headers.clone()),
                "/edit-tag" => {
                    let a = body_form.iter().find(|(k, _)| k == "a").map(|(_, v)| v.clone());
                    let r = body_form.iter().find(|(k, _)| k == "r").map(|(_, v)| v.clone());
                    for id in ids_of(&body_form) {
                        let first_usec =
                            st.arts.get(&id).and_then(|a| st.subs.iter().find(|s| s.id == a.feed).map(|s| s.first_usec)).unwrap_or(0);
                        if let Some(art) = st.arts.get_mut(&id) {
                            for (tag, val) in [(&a, true), (&r, false)] {
                                let Some(tag) = tag else { continue };
                                match tag.as_str() {
                                    "user/-/state/com.google/read" => {
                                        // Marking older articles unread is silently ignored.
                                        if val || art.ts >= first_usec {
                                            art.read = val;
                                        }
                                    }
                                    "user/-/state/com.google/starred" => art.starred = val,
                                    "user/-/state/com.google/like" => art.like = val,
                                    t => {
                                        if let Some(l) = t.strip_prefix("user/-/label/") {
                                            if val {
                                                art.labels.insert(l.to_string());
                                            } else {
                                                art.labels.remove(l);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    ok("OK".into(), headers.clone())
                }
                p if p == "/stream/items/ids" || p.starts_with("/stream/contents/") => {
                    let contents = path.starts_with("/stream/contents/");
                    let stream = if contents {
                        percent_encoding::percent_decode_str(path.trim_start_matches("/stream/contents/")).decode_utf8_lossy().into_owned()
                    } else {
                        qget("s").unwrap_or_default()
                    };
                    let mut list: Vec<SArt> = st.arts.values().cloned().collect();
                    list.sort_by(|a, b| b.ts.cmp(&a.ts).then(b.id.cmp(&a.id)));
                    let subs = st.subs.clone();
                    list.retain(|a| match stream.as_str() {
                        "user/-/state/com.google/starred" => a.starred,
                        "user/-/state/com.google/reading-list" => true,
                        "user/-/state/com.google/like" => a.like,
                        "user/-/state/com.google/annotated" => !a.annotations.is_empty(),
                        s if s.starts_with("feed/") => a.feed == s,
                        s if s.starts_with("user/-/label/") => {
                            let l = s.trim_start_matches("user/-/label/");
                            a.labels.contains(l) || subs.iter().any(|x| x.id == a.feed && x.folders.iter().any(|f| f == l))
                        }
                        _ => false,
                    });
                    if qget("xt").as_deref() == Some("user/-/state/com.google/read") {
                        list.retain(|a| !a.read);
                    }
                    if qget("it").as_deref() == Some("user/-/state/com.google/starred") {
                        list.retain(|a| a.starred);
                    }
                    let n = qget("n").and_then(|n| n.parse::<usize>().ok()).unwrap_or(20).min(st.page_cap);
                    let off = qget("c").and_then(|c| c.strip_prefix('c').and_then(|n| n.parse::<usize>().ok())).unwrap_or(0);
                    let end = (off + n).min(list.len());
                    let page = &list[off.min(list.len())..end];
                    let cont = if loop_token {
                        Some("same".to_string())
                    } else if end < list.len() {
                        Some(format!("c{end}"))
                    } else {
                        None
                    };
                    let mut body = if contents {
                        serde_json::json!({"items": items_json(page, &subs, qget("annotations").as_deref() == Some("1"))})
                    } else {
                        let refs: Vec<_> = page
                            .iter()
                            .map(|a| serde_json::json!({"id": (a.id as i64).to_string(), "timestampUsec": a.ts.to_string()}))
                            .collect();
                        serde_json::json!({"items": [], "itemRefs": refs})
                    };
                    if let Some(c) = cont {
                        body["continuation"] = c.into();
                    }
                    ok(body.to_string(), headers.clone())
                }
                "/subscription/list" => {
                    let subs: Vec<_> = st
                        .subs
                        .iter()
                        .map(|s| {
                            serde_json::json!({"id": s.id, "title": s.title, "url": s.url, "htmlUrl": "https://blog.example/", "sortid": "00000001", "firstitemmsec": s.first_usec.to_string(),
                                "categories": s.folders.iter().map(|f| serde_json::json!({"id": format!("user/42/label/{f}"), "label": f})).collect::<Vec<_>>()})
                        })
                        .collect();
                    ok(serde_json::json!({"subscriptions": subs}).to_string(), headers.clone())
                }
                "/tag/list" => {
                    let mut tags: Vec<_> =
                        st.tags.iter().map(|(id, kind)| serde_json::json!({"id": format!("user/42/label/{id}"), "type": kind})).collect();
                    let mut folders = BTreeSet::new();
                    for s in &st.subs {
                        for f in &s.folders {
                            folders.insert(f.clone());
                        }
                    }
                    for f in folders {
                        if !st.tags.iter().any(|(id, _)| *id == f) {
                            tags.push(serde_json::json!({"id": format!("user/42/label/{f}"), "type": "folder"}));
                        }
                    }
                    ok(serde_json::json!({"tags": tags}).to_string(), headers.clone())
                }
                "/unread-count" => {
                    // As the real API: every feed and folder/tag (even at zero), the reading
                    // list with its newest unread article, and the starred total.
                    let unread: Vec<&SArt> = st.arts.values().filter(|a| !a.read).collect();
                    let newest = unread.iter().map(|a| a.ts).max().unwrap_or(0);
                    let entry = |id: String, n: usize, newest: i64| serde_json::json!({"id": id, "count": n, "newestItemTimestampUsec": newest.to_string()});
                    let mut list = vec![
                        entry("user/42/state/com.google/reading-list".into(), unread.len(), newest),
                        entry("user/42/state/com.google/starred".into(), st.arts.values().filter(|a| a.starred).count(), 0),
                    ];
                    let mut labels = BTreeSet::new();
                    for s in &st.subs {
                        list.push(entry(s.id.clone(), unread.iter().filter(|a| a.feed == s.id).count(), 0));
                        labels.extend(s.folders.iter().cloned());
                    }
                    labels.extend(st.tags.iter().map(|(id, _)| id.clone()));
                    for l in labels {
                        list.push(entry(format!("user/42/label/{l}"), 0, 0));
                    }
                    ok(serde_json::json!({"max": 1000, "unreadcounts": list}).to_string(), headers.clone())
                }
                "/preference/stream/list" => ok(r#"{"streamprefs":{}}"#.into(), headers.clone()),
                "/subscription/edit" => {
                    let get = |k: &str| body_form.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone());
                    let stream = get("s").unwrap();
                    match get("ac").as_deref() {
                        Some("unsubscribe") => st.subs.retain(|s| s.id != stream),
                        Some(_) => {
                            if let Some(sub) = st.subs.iter_mut().find(|s| s.id == stream) {
                                if let Some(t) = get("t") {
                                    sub.title = t;
                                }
                                if let Some(a) = get("a") {
                                    let f = a.trim_start_matches("user/-/label/").to_string();
                                    if !sub.folders.contains(&f) {
                                        sub.folders.push(f);
                                    }
                                }
                                if let Some(r) = get("r") {
                                    let f = r.trim_start_matches("user/-/label/");
                                    sub.folders.retain(|x| x != f);
                                }
                            }
                        }
                        None => {}
                    }
                    ok("OK".into(), headers.clone())
                }
                "/subscription/quickadd" => {
                    let feed = body_form.iter().find(|(k, _)| k == "quickadd").map(|(_, v)| v.clone()).unwrap();
                    let id = format!("feed/{feed}");
                    if !st.subs.iter().any(|s| s.id == id) {
                        st.subs.push(SSub { id: id.clone(), title: feed.clone(), url: feed.clone(), folders: vec![], first_usec: 0 });
                    }
                    ok(serde_json::json!({"query": feed, "numResults": 1, "streamId": id}).to_string(), headers.clone())
                }
                "/rename-tag" => {
                    let get = |k: &str| body_form.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone()).unwrap();
                    let src = get("s").trim_start_matches("user/-/label/").to_string();
                    let dest = get("dest").trim_start_matches("user/-/label/").to_string();
                    for s in st.subs.iter_mut() {
                        for f in s.folders.iter_mut() {
                            if *f == src {
                                *f = dest.clone();
                            }
                        }
                    }
                    ok("OK".into(), headers.clone())
                }
                "/disable-tag" => {
                    let src =
                        body_form.iter().find(|(k, _)| k == "s").map(|(_, v)| v.trim_start_matches("user/-/label/").to_string()).unwrap();
                    for s in st.subs.iter_mut() {
                        s.folders.retain(|f| *f != src);
                    }
                    ok("OK".into(), headers.clone())
                }
                "/active_search/create" => {
                    st.tags.push(("search1".into(), "active_search".into()));
                    ok(r#"{"id":"user/42/search/1","title":"t"}"#.into(), headers.clone())
                }
                other => Ok(HttpResponse { status: 404, headers: vec![], body: other.as_bytes().to_vec() }),
            }
        })();
        if lost {
            return Err(Error::UnknownOutcome("response lost".into()));
        }
        result
    }
}
