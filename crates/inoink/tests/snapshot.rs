mod common;

use common::fake::{Fake, Kind, SSub};
use common::*;
use inoink::api::*;
use inoink::cache::{Cache, CacheConfig};
use inoink::content::images::{Fetched, Fetcher};
use inoink::model::*;
use inoink::snapshot::*;
use inoink::store::Store;
use inoink::{Error, Result};
use serde_json::json;
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use tempfile::tempdir;
use url::Url;

fn config() -> AppConfig {
    let mut cfg = AppConfig::new("snapshot-client", "snapshot-secret", "http://localhost:8080/cb");
    cfg.base_url = "https://example.test/reader/api/0".into();
    cfg.token_url = "https://example.test/oauth2/token".into();
    cfg
}

fn read_tokens(expiry: i64) -> Arc<MemTokens> {
    let store = Arc::new(MemTokens::default());
    *store.tokens.lock().unwrap() = Some(Tokens { scope: "read".into(), ..tokens(expiry) });
    store
}

fn request(method: Method, url: &str, body: Option<Body>) -> HttpRequest {
    HttpRequest { method, url: url.into(), headers: vec![("Authorization".into(), "Bearer private-token".into())], body }
}

fn grant(value: &str) -> Option<Body> {
    Some(Body::Form(vec![("grant_type".into(), value.into()), ("client_secret".into(), "private-secret".into())]))
}

fn assert_denied<T>(result: Result<T>) {
    match result {
        Err(Error::PermissionDenied(message)) => {
            assert!(!message.contains("private"));
            assert!(!message.contains("https://"));
            assert!(!message.contains("grant_type"));
        }
        _ => panic!("expected a fixed permission denial"),
    }
}

#[test]
fn read_allowlist_is_exact_and_supports_read_queries_and_normalized_origins() {
    let mock = Mock::new();
    *mock.fallback.lock().unwrap() = Some(Box::new(|_| Ok(resp(200, r#"{"items":[]}"#))));
    let guarded = ReadOnlyTransport::new(mock.clone(), &config()).unwrap();
    for path in [
        "/user-info",
        "/subscription/list",
        "/tag/list?types=1&counts=1",
        "/unread-count?output=json",
        "/preference/stream/list",
        "/stream/contents/user%2F-%2Fstate%2Fcom.google%2Fstarred?n=100&annotations=1&summaries=1&c=next",
        "/stream/contents/feed%2Fhttps%3A%2F%2Fpublisher.example%2Frss",
    ] {
        guarded.send(&request(Method::Get, &format!("{}{path}", config().base_url), None)).unwrap();
    }
    guarded.send(&request(Method::Get, "https://EXAMPLE.test:443/reader/api/0/user-info", None)).unwrap();
    assert_eq!(mock.count(), 8);
}

#[test]
fn forbidden_methods_origins_paths_and_bodies_never_reach_transport() {
    let mock = Mock::new();
    let guarded = ReadOnlyTransport::new(mock.clone(), &config()).unwrap();
    let base = config().base_url;
    let paths = [
        "/active_search/delete?id=private-search",
        "/edit-tag",
        "/subscription/quickadd",
        "/subscription/edit",
        "/rename-tag",
        "/disable-tag",
        "/preference/stream/set",
        "/mark-all-as-read",
        "/active_search/create",
        "/stream/items/ids",
        "/stream/items/contents",
        "/user-info/extra",
        "/user-info/",
        "/user-info.json",
        "/subscription/listing",
        "/stream/contents",
        "/stream/contents/",
        "/stream/contents-evil/private",
        "/stream/contents/../../../active_search/delete",
        "/stream/contents/%2E%2e/%2e%2E/active_search/delete",
        "/stream/contents/a%2F..%2F..%2Factive_search/delete",
        "/stream/contents/a%5C..%5Cactive_search/delete",
        "/stream/contents/%252e%252e%252f%252e%252e%252factive_search/delete",
        "/stream/contents/%255c..%255cactive_search/delete",
        "/%75ser-info",
        "/user-info#private-fragment",
    ];
    for path in paths {
        for method in [Method::Get, Method::Post] {
            assert_denied(guarded.send(&request(method, &format!("{base}{path}"), None)));
        }
    }
    for url in [
        "http://example.test/reader/api/0/user-info",
        "https://example.test:444/reader/api/0/user-info",
        "https://example.test.evil/reader/api/0/user-info",
        "https://evil.test/reader/api/0/user-info",
        "https://example.test/reader/api/00/user-info",
        "https://example.test/reader/api/0evil/user-info",
        "https://example.test/elsewhere/user-info",
        "https://user@example.test/reader/api/0/user-info",
        "https://user:private@example.test/reader/api/0/user-info",
        "https://@example.test/reader/api/0/user-info",
        "https:////@example.test/reader/api/0/user-info",
        "https:///example.test/reader/api/0/user-info",
        "https://example.test/reader/api/0/../0/user-info",
        "https://example.test/reader/api/0/stream/contents/a/../../active_search/delete",
        "https://example.test/reader/api/0\\user-info",
        "https://example.test/reader/api/0/user-info\n",
        "file:///reader/api/0/user-info",
        "not a URL",
    ] {
        assert_denied(guarded.send(&request(Method::Get, url, None)));
    }
    for body in [None, Some(Body::Form(vec![])), Some(Body::Json("private-body".into()))] {
        assert_denied(guarded.send(&request(Method::Post, &format!("{base}/user-info"), body.clone())));
        if body.is_some() {
            assert_denied(guarded.send(&request(Method::Get, &format!("{base}/user-info"), body)));
        }
    }
    assert_eq!(mock.count(), 0);
}

#[test]
fn token_endpoint_allows_only_exact_post_form_with_one_known_grant() {
    let cfg = config();
    let mock = Mock::new();
    let guarded = ReadOnlyTransport::new(mock.clone(), &cfg).unwrap();
    for value in ["authorization_code", "refresh_token"] {
        mock.push_ok();
        guarded.send(&request(Method::Post, &cfg.token_url, grant(value))).unwrap();
    }
    for value in ["", "password", "client_credentials", "refresh_token ", "authorization_code refresh_token"] {
        assert_denied(guarded.send(&request(Method::Post, &cfg.token_url, grant(value))));
    }
    for body in [
        None,
        Some(Body::Json(r#"{"grant_type":"refresh_token"}"#.into())),
        Some(Body::Form(vec![])),
        Some(Body::Form(vec![("grant_type".into(), "refresh_token".into()), ("grant_type".into(), "refresh_token".into())])),
        Some(Body::Form(vec![("grant_type".into(), "authorization_code".into()), ("grant_type".into(), "password".into())])),
    ] {
        assert_denied(guarded.send(&request(Method::Post, &cfg.token_url, body)));
    }
    for url in [
        format!("{}?", cfg.token_url),
        format!("{}?grant_type=refresh_token", cfg.token_url),
        format!("{}#", cfg.token_url),
        format!("{}/extra", cfg.token_url),
        cfg.token_url.replace("https:", "http:"),
        cfg.token_url.replace("example.test", "evil.test"),
        cfg.token_url.replace("example.test", "user@example.test"),
        cfg.token_url.replace("example.test", "example.test:444"),
    ] {
        assert_denied(guarded.send(&request(Method::Post, &url, grant("refresh_token"))));
    }
    assert_denied(guarded.send(&request(Method::Get, &cfg.token_url, None)));
    assert_eq!(mock.count(), 2);
}

#[test]
fn invalid_configuration_is_rejected_without_echoing_it() {
    for url in [
        "not-a-url-private",
        "https://user:private@example.test/api",
        "https://example.test/api?private",
        "https://example.test/api#private",
    ] {
        let mut cfg = config();
        cfg.base_url = url.into();
        assert_denied(ReadOnlyTransport::new(Mock::new(), &cfg));
        let mut cfg = config();
        cfg.token_url = url.into();
        assert_denied(ReadOnlyTransport::new(Mock::new(), &cfg));
    }
}

#[test]
fn only_read_scope_is_accepted_on_load_and_save() {
    let inner = read_tokens(10_000);
    let guarded = ReadOnlyTokens::new(inner.clone());
    for scope in ["read", " read\n\tread  "] {
        let t = Tokens { scope: scope.into(), ..tokens(10_000) };
        require_read_scope(&t).unwrap();
        guarded.save(&t).unwrap();
        assert_eq!(guarded.load().unwrap(), Some(t));
    }
    let saved = inner.saves.lock().unwrap().len();
    for scope in ["", " \t", "read write", "write", "Read", "read unknown", "read,write", "private-scope"] {
        let t = Tokens { scope: scope.into(), ..tokens(10_000) };
        assert_denied(require_read_scope(&t));
        assert_denied(guarded.save(&t));
        assert_eq!(inner.saves.lock().unwrap().len(), saved);
        *inner.tokens.lock().unwrap() = Some(t);
        assert_denied(guarded.load());
    }
    *inner.tokens.lock().unwrap() = None;
    assert_eq!(guarded.load().unwrap(), None);
}

fn guarded_client(mock: Arc<Mock>, tokens: Arc<MemTokens>) -> ApiClient {
    let cfg = config();
    let transport = Arc::new(ReadOnlyTransport::new(mock, &cfg).unwrap());
    ApiClient::new(cfg, transport, FixedClock::new(1_000_000), Arc::new(ReadOnlyTokens::new(tokens))).unwrap()
}

#[test]
fn expired_tokens_and_401_retry_persist_rotations_before_use() {
    for expired in [false, true] {
        let mock = Mock::new();
        let tokens = read_tokens(if expired { 0 } else { 2_000_000 });
        if !expired {
            mock.push(Ok(resp(401, "")));
        }
        mock.push_json(json!({"access_token":"rotated-access", "refresh_token":"rotated-refresh", "expires_in":3600, "scope":"read"}));
        let saved = tokens.clone();
        *mock.fallback.lock().unwrap() = Some(Box::new(move |req| {
            let rotations = saved.saves.lock().unwrap();
            assert_eq!(rotations.len(), 1);
            assert_eq!(rotations[0].refresh_token, "rotated-refresh");
            assert!(req.headers.contains(&("Authorization".into(), "Bearer rotated-access".into())));
            Ok(resp(200, r#"{"userId":"42"}"#))
        }));
        guarded_client(mock.clone(), tokens.clone()).user_info().unwrap();
        assert_eq!(tokens.saves.lock().unwrap().len(), 1);
        assert_eq!(mock.all().iter().filter(|r| r.method == Method::Post).count(), 1);
        assert_eq!(mock.count(), if expired { 2 } else { 3 });
        assert_eq!(form_get(&mock.all()[usize::from(!expired)], "grant_type"), ["refresh_token"]);
    }
}

#[test]
fn expanded_or_missing_refresh_scope_is_never_saved_or_used() {
    for scope in [None, Some(""), Some("read write"), Some("unknown")] {
        for expired in [false, true] {
            let mock = Mock::new();
            let tokens = read_tokens(if expired { 0 } else { 2_000_000 });
            let previous = tokens.load().unwrap();
            if !expired {
                mock.push(Ok(resp(401, "")));
            }
            let mut reply = json!({"access_token":"private-expanded", "refresh_token":"private-rotated", "expires_in":3600});
            if let Some(scope) = scope {
                reply["scope"] = scope.into();
            }
            mock.push_json(reply);
            assert_denied(guarded_client(mock.clone(), tokens.clone()).user_info());
            assert_eq!(tokens.load().unwrap(), previous);
            assert!(tokens.saves.lock().unwrap().is_empty());
            assert_eq!(mock.count(), if expired { 1 } else { 2 });
            assert!(mock.all().iter().all(|r| !r.headers.iter().any(|(_, v)| v.contains("private-expanded"))));
        }
    }
}

#[test]
fn production_write_apis_are_guarded_but_normal_client_is_unchanged() {
    let mock = Mock::new();
    let client = guarded_client(mock.clone(), read_tokens(2_000_000));
    let stream = StreamId::label("private-label");
    assert_denied(client.edit_tag(&[ArticleId(1)], Some(&Membership::Read), None));
    assert_denied(client.quickadd("https://publisher.example/rss"));
    assert_denied(client.edit_subscription(&SubscriptionEdit {
        stream: stream.clone(),
        action: SubAction::Unsubscribe,
        title: None,
        add_folder: None,
        remove_folder: None,
    }));
    assert_denied(client.rename_tag(&stream, "new"));
    assert_denied(client.disable_tag(&stream));
    assert_denied(client.set_stream_order(&stream, &["00000001".into()]));
    assert_denied(client.mark_all_read(&stream, 100));
    assert_denied(client.create_active_search(&ActiveSearchSpec {
        term: "test".into(),
        filter_type: FilterType::Public,
        filter_id: None,
        article_ids: vec![],
        search_filters: None,
        search_match: None,
        search_feed_popularity: None,
    }));
    assert_denied(client.delete_active_search(&stream));
    assert_eq!(mock.count(), 0);
    let normal = harness();
    normal.mock.push_ok();
    normal.client.edit_tag(&[ArticleId(1)], Some(&Membership::Read), None).unwrap();
    normal.mock.push_ok();
    normal.client.delete_active_search(&stream).unwrap();
    assert_eq!(normal.mock.count(), 2);
}

#[derive(Default)]
struct Publisher {
    calls: Mutex<Vec<(String, usize)>>,
}

impl Fetcher for Publisher {
    // This interface has no OAuth headers; the snapshot never passes API requests here.
    fn get(&self, url: &Url, max: usize) -> Result<Fetched> {
        self.calls.lock().unwrap().push((url.to_string(), max));
        assert!(!url.as_str().contains("private"));
        assert!(!url.as_str().contains("acc"));
        Ok(Fetched { status: 404, ..Default::default() })
    }
}

fn run(output: &Path, server: Arc<dyn Transport>, publisher: Arc<dyn Fetcher>) -> Result<SnapshotReport> {
    download(output, config(), server, FixedClock::new(1_700_000_000), read_tokens(2_000_000_000), publisher)
}

fn query_all() -> ArticleQuery {
    ArticleQuery { limit: 200, ..Default::default() }
}

fn assert_artifacts_private_and_no_credentials(output: &Path) {
    assert_eq!(fs::metadata(output).unwrap().permissions().mode() & 0o777, 0o700);
    fn walk(path: &Path) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name();
            let name = name.to_str().unwrap();
            assert!(!["credentials.json", "share.json", "device.key", "provision.json", "provision.age", "share.age"].contains(&name));
            if entry.path().is_dir() {
                walk(&entry.path());
            } else {
                let contents = fs::read(entry.path()).unwrap();
                for secret in ["snapshot-secret", "snapshot-client", "snapshot-credential-sentinel"] {
                    assert!(!contents.windows(secret.len()).any(|s| s == secret.as_bytes()));
                }
            }
        }
    }
    walk(output);
}

#[test]
fn downloads_realistic_deduplicated_offline_content_without_mutating_server() {
    let dir = tempdir().unwrap();
    let output = dir.path().join("data");
    let fake = Fake::new();
    for id in 1..=4 {
        fake.art(id, 1_700_000_000_000_000 + id as i64, "feed/https://blog.example/rss");
    }
    fake.with(|s| {
        s.subs.push(SSub {
            id: "feed/https://blog.example/rss".into(),
            title: "Blog".into(),
            url: "https://blog.example/rss".into(),
            folders: vec!["Folder".into()],
            first_usec: 0,
        });
        s.tags.push(("Tag".into(), "tag".into()));
        s.arts.get_mut(&2).unwrap().starred = true;
        let saved = s.arts.get_mut(&3).unwrap();
        saved.starred = true;
        saved.read = true;
        saved.labels.insert("Tag".into());
        saved.annotations.push(json!({"id":1, "start":0, "end":4, "text":"Body", "note":"Note", "added_on":1700000000}));
        s.arts.get_mut(&4).unwrap().read = true; // Neither unread nor starred: not exported.
    });
    let before = fake.with(|s| format!("{:?}{:?}{:?}", s.arts, s.subs, s.tags));
    let recording = Mock::new();
    let server = fake.clone();
    *recording.fallback.lock().unwrap() = Some(Box::new(move |req| {
        if req.url.ends_with("/preference/stream/list") {
            return Ok(resp(
                200,
                r#"{"streamprefs":{"user/42/state/com.google/root":[{"id":"subscription-ordering","value":"00000001"}]}}"#,
            ));
        }
        server.send(req)
    }));
    let publisher = Arc::new(Publisher::default());
    let credentials = read_tokens(2_000_000_000);
    credentials.tokens.lock().unwrap().as_mut().unwrap().access_token = "snapshot-credential-sentinel".into();
    fake.with(|s| s.valid_access = "snapshot-credential-sentinel".into());
    assert_eq!(
        download(&output, config(), recording.clone(), FixedClock::new(1_700_000_000), credentials, publisher.clone()).unwrap(),
        SnapshotReport { articles: 3, incomplete_images: 0 }
    );
    assert_eq!(before, fake.with(|s| format!("{:?}{:?}{:?}", s.arts, s.subs, s.tags)));
    assert_eq!(fake.with(|s| s.usage[1]), 0);
    let store = Store::open(&output.join("inoink.db"), "local").unwrap();
    assert_eq!(store.account(), "local");
    assert_eq!(store.get_meta("preview_source").unwrap().as_deref(), Some("inoreader"));
    assert_eq!(store.count_articles(&query_all()).unwrap(), 3);
    assert!(store.effective(ArticleId(3), &Membership::Read).unwrap());
    assert!(store.effective(ArticleId(3), &Membership::Starred).unwrap());
    assert!(!store.effective(ArticleId(2), &Membership::Read).unwrap());
    assert!(store.effective(ArticleId(2), &Membership::Starred).unwrap());
    assert!(!store.effective(ArticleId(1), &Membership::Starred).unwrap());
    assert_eq!(store.effective_labels(ArticleId(3)).unwrap(), ["Tag"]);
    assert_eq!(store.remote_annotations(ArticleId(3)).unwrap().len(), 1);
    assert_eq!(store.subscriptions().unwrap()[0].folders, ["Folder"]);
    assert_eq!(store.stream_order("user/-/state/com.google/root").unwrap().as_deref(), Some("00000001"));
    assert_eq!(store.pending_count().unwrap(), 0);
    assert!(!store.org_pending().unwrap());
    assert!(store.org_ops().unwrap().is_empty());
    assert_eq!(store.share_counts().unwrap(), (0, 0));
    assert!(store.pending_shares(10).unwrap().is_empty());
    let cache = Cache::new(&output.join("cache"), "local", CacheConfig::default());
    let verified = cache.verify(&store).unwrap();
    assert_eq!(verified.checked, 3);
    assert!(verified.problems.is_empty());
    for id in [ArticleId(1), ArticleId(2), ArticleId(3)] {
        assert!(store.download_state(id).unwrap().readable_offline());
        let version = store.latest_version(id).unwrap().unwrap();
        assert!(fs::read_to_string(cache.version_path(id, version.version)).unwrap().contains("Body of article"));
    }
    let marker: serde_json::Value = serde_json::from_slice(&fs::read(output.join(SANDBOX_MARKER)).unwrap()).unwrap();
    assert_eq!(
        marker,
        json!({"format":1,"source":"inoreader","downloaded_at":1700000000,"articles":3,"incomplete_images":0,"per_stream_limit":100})
    );
    let requests = recording.all();
    assert_eq!(requests.len(), 7);
    assert!(requests.iter().all(|r| r.method == Method::Get && r.body.is_none()));
    assert_eq!(requests.iter().filter(|r| r.url.ends_with("/user-info")).count(), 1);
    let contents: Vec<_> = requests.iter().filter(|r| r.url.contains("/stream/contents/")).collect();
    assert_eq!(contents.len(), 2);
    for req in contents {
        assert_eq!(q1(&req.url, "n").as_deref(), Some("100"));
        assert_eq!(q1(&req.url, "annotations").as_deref(), Some("1"));
        assert_eq!(q1(&req.url, "summaries").as_deref(), Some("1"));
        assert_eq!(q1(&req.url, "c"), None);
        assert_eq!(q1(&req.url, "r"), None);
    }
    assert!(!publisher.calls.lock().unwrap().is_empty()); // Stub extraction matches the device.
    assert_artifacts_private_and_no_credentials(&output);
    let other = store.for_account("42").unwrap();
    assert_eq!(other.count_articles(&query_all()).unwrap(), 0);
}

#[test]
fn exactly_one_page_per_stream_ignores_both_continuations_and_caps_at_200() {
    let dir = tempdir().unwrap();
    let fake = Fake::new();
    for id in 1..=240 {
        fake.art(id, 1_700_000_000_000_000 + id as i64, "feed/blog");
        if id > 120 {
            fake.with(|s| {
                let art = s.arts.get_mut(&id).unwrap();
                art.starred = true;
                art.read = true;
            });
        }
    }
    let recording = Mock::new();
    let server = fake.clone();
    let continuations = Arc::new(Mutex::new(Vec::new()));
    let seen = continuations.clone();
    *recording.fallback.lock().unwrap() = Some(Box::new(move |req| {
        let response = server.send(req)?;
        if req.url.contains("/stream/contents/") {
            let value: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
            seen.lock().unwrap().push(value["continuation"].as_str().unwrap().to_string());
        }
        Ok(response)
    }));
    let output = dir.path().join("data");
    assert_eq!(run(&output, recording.clone(), Arc::new(Publisher::default())).unwrap().articles, 200);
    assert_eq!(continuations.lock().unwrap().len(), 2);
    let requests = recording.all();
    assert_eq!(requests.iter().filter(|r| r.url.contains("/stream/contents/")).count(), 2);
    assert!(requests.iter().all(|r| q1(&r.url, "c").is_none()));
    let store = Store::open(&output.join("inoink.db"), "local").unwrap();
    assert_eq!(store.count_articles(&query_all()).unwrap(), 200);
    assert!(!store.has_article(ArticleId(1)).unwrap());
    assert!(!store.has_article(ArticleId(121)).unwrap());
    assert!(store.has_article(ArticleId(21)).unwrap());
    assert!(store.has_article(ArticleId(240)).unwrap());
    assert_eq!(fake.with(|s| s.usage[1]), 0);
}

#[test]
fn missing_images_are_readable_and_counted_per_article() {
    let dir = tempdir().unwrap();
    let fake = Fake::new();
    fake.art(1, 1_700_000_000_000_000, "feed/blog");
    fake.with(|s| {
        s.arts.get_mut(&1).unwrap().html =
            "<p>Readable text</p><img src='https://images.example/a.png'/><img src='https://images.example/b.png'/>".into()
    });
    let output = dir.path().join("data");
    let publisher = Arc::new(Publisher::default());
    assert_eq!(run(&output, fake, publisher.clone()).unwrap(), SnapshotReport { articles: 1, incomplete_images: 1 });
    let store = Store::open(&output.join("inoink.db"), "local").unwrap();
    assert_eq!(store.download_state(ArticleId(1)).unwrap(), DownloadState::MissingImages);
    assert!(publisher.calls.lock().unwrap().iter().any(|(url, max)| url.contains("images.example") && *max > 0));
    assert!(Cache::new(&output.join("cache"), "local", CacheConfig::default()).verify(&store).unwrap().problems.is_empty());
    assert!(is_sandbox(&output));
}

#[test]
fn empty_account_publishes_a_valid_empty_snapshot() {
    let dir = tempdir().unwrap();
    let output = dir.path().join("data");
    assert_eq!(run(&output, Fake::new(), Arc::new(Publisher::default())).unwrap(), SnapshotReport { articles: 0, incomplete_images: 0 });
    assert!(is_sandbox(&output));
    assert_artifacts_private_and_no_credentials(&output);
}

struct NoTokens;
impl TokenStore for NoTokens {
    fn load(&self) -> Result<Option<Tokens>> {
        panic!("preexisting output must be refused before token load")
    }
    fn save(&self, _: &Tokens) -> Result<()> {
        panic!("unexpected token save")
    }
}

#[test]
fn existing_output_including_symlinks_is_refused_before_tokens_or_network() {
    let dir = tempdir().unwrap();
    let existing = dir.path().join("existing");
    fs::create_dir(&existing).unwrap();
    fs::write(existing.join("keep"), "untouched").unwrap();
    let file = dir.path().join("file");
    fs::write(&file, "untouched").unwrap();
    let link = dir.path().join("link");
    symlink(&existing, &link).unwrap();
    let dangling = dir.path().join("dangling");
    symlink(dir.path().join("missing"), &dangling).unwrap();
    let mock = Mock::new();
    for output in [&existing, &file, &link, &dangling] {
        assert!(matches!(
            download(output, config(), mock.clone(), FixedClock::new(1_700_000_000), Arc::new(NoTokens), Arc::new(Publisher::default())),
            Err(Error::InvalidRequest(_))
        ));
    }
    assert_eq!(mock.count(), 0);
    assert_eq!(fs::read_to_string(existing.join("keep")).unwrap(), "untouched");
    assert_eq!(fs::read_to_string(file).unwrap(), "untouched");
}

#[test]
fn bad_initial_scope_fails_before_creating_output_or_network() {
    let dir = tempdir().unwrap();
    let output = dir.path().join("data");
    let mock = Mock::new();
    let tokens = read_tokens(2_000_000_000);
    tokens.tokens.lock().unwrap().as_mut().unwrap().scope = "read write".into();
    assert_denied(download(&output, config(), mock.clone(), FixedClock::new(1_700_000_000), tokens, Arc::new(Publisher::default())));
    assert!(!output.exists());
    assert_eq!(mock.count(), 0);
}

#[test]
fn api_failure_never_publishes_marker() {
    for endpoint in ["/user-info", "/unread-count", "/subscription/list", "/tag/list", "/preference/stream/list", "/stream/contents/"] {
        let dir = tempdir().unwrap();
        let output = dir.path().join("data");
        let fake = Fake::new();
        fake.fail(endpoint, Kind::Status(500), 1);
        assert!(run(&output, fake, Arc::new(Publisher::default())).is_err());
        assert!(!is_sandbox(&output));
    }
}

#[test]
fn malformed_or_oversized_content_page_aborts_without_caching() {
    for items in [json!([{"summary":{"content":"unreadable"}}]), json!(vec![json!({"id":"1"}); 101])] {
        let dir = tempdir().unwrap();
        let output = dir.path().join("data");
        let fake = Fake::new();
        let recording = Mock::new();
        *recording.fallback.lock().unwrap() = Some(Box::new(move |req| {
            if req.url.contains("/stream/contents/") {
                return Ok(resp(200, &json!({"items":items}).to_string()));
            }
            fake.send(req)
        }));
        let publisher = Arc::new(Publisher::default());
        assert!(run(&output, recording, publisher.clone()).is_err());
        assert!(!is_sandbox(&output));
        assert!(publisher.calls.lock().unwrap().is_empty());
    }
}

struct CorruptingPublisher {
    output: std::path::PathBuf,
}
impl Fetcher for CorruptingPublisher {
    fn get(&self, url: &Url, _: usize) -> Result<Fetched> {
        // Corrupt an earlier article while a later article's images are being fetched.
        if url.host_str() == Some("images.example") {
            let cache = Cache::new(&self.output.join("cache"), "local", CacheConfig::default());
            fs::write(cache.version_path(ArticleId(1), 1), "corrupt").unwrap();
        }
        Ok(Fetched { status: 404, ..Default::default() })
    }
}

#[test]
fn cache_verification_failure_aborts_publication() {
    let dir = tempdir().unwrap();
    let output = dir.path().join("data");
    let fake = Fake::new();
    fake.art(1, 1_700_000_001_000_000, "feed/blog");
    fake.art(2, 1_700_000_000_000_000, "feed/blog");
    fake.with(|s| s.arts.get_mut(&2).unwrap().html = "<p>Body</p><img src='https://images.example/missing.png'/>".into());
    let publisher = Arc::new(CorruptingPublisher { output: output.clone() });
    assert!(matches!(run(&output, fake, publisher), Err(Error::Data(message)) if message == "Snapshot cache verification failed."));
    assert!(!is_sandbox(&output));
}

struct FailingSave {
    initial: Tokens,
}
impl TokenStore for FailingSave {
    fn load(&self) -> Result<Option<Tokens>> {
        Ok(Some(self.initial.clone()))
    }
    fn save(&self, _: &Tokens) -> Result<()> {
        Err(Error::Storage("cannot persist rotation".into()))
    }
}

#[test]
fn failed_rotation_persistence_prevents_using_the_new_access_token() {
    let cfg = config();
    let mock = Mock::new();
    mock.push_json(json!({"access_token":"new-access", "refresh_token":"new-refresh", "scope":"read", "expires_in":3600}));
    let transport = Arc::new(ReadOnlyTransport::new(mock.clone(), &cfg).unwrap());
    let store = Arc::new(ReadOnlyTokens::new(Arc::new(FailingSave { initial: Tokens { scope: "read".into(), ..tokens(0) } })));
    let client = ApiClient::new(cfg, transport, FixedClock::new(1_000_000), store).unwrap();
    assert!(matches!(client.user_info(), Err(Error::Storage(_))));
    assert_eq!(mock.count(), 1);
    assert_eq!(mock.last().method, Method::Post);
    assert!(mock.last().headers.iter().all(|(key, _)| key != "Authorization"));
}

struct UncacheablePublisher;
impl Fetcher for UncacheablePublisher {
    fn get(&self, _: &Url, _: usize) -> Result<Fetched> {
        Err(Error::Storage("content storage unavailable".into()))
    }
}

#[test]
fn article_that_cannot_be_cached_aborts_instead_of_publishing_unreadable_content() {
    let dir = tempdir().unwrap();
    let output = dir.path().join("data");
    let fake = Fake::new();
    fake.art(1, 1_700_000_000_000_000, "feed/blog");
    assert!(matches!(run(&output, fake, Arc::new(UncacheablePublisher)), Err(Error::Storage(_))));
    assert!(!is_sandbox(&output));
    let store = Store::open(&output.join("inoink.db"), "local").unwrap();
    assert!(!store.download_state(ArticleId(1)).unwrap().readable_offline());
    assert!(store.latest_version(ArticleId(1)).unwrap().is_none());
}

#[test]
fn all_responses_including_oauth_are_size_bounded() {
    let cfg = config();
    let mock = Mock::new();
    let transport = ReadOnlyTransport::new(mock.clone(), &cfg).unwrap();
    for req in
        [request(Method::Get, &format!("{}/user-info", cfg.base_url), None), request(Method::Post, &cfg.token_url, grant("refresh_token"))]
    {
        mock.push(Ok(HttpResponse { status: 200, body: vec![b'x'; MAX_BODY + 1], ..Default::default() }));
        assert!(matches!(transport.send(&req), Err(Error::Data(message)) if message == "Snapshot response is too large."));
    }
}

#[test]
fn failed_second_stream_does_not_publish_a_partial_snapshot() {
    let dir = tempdir().unwrap();
    let output = dir.path().join("data");
    let fake = Fake::new();
    fake.art(1, 1_700_000_000_000_000, "feed/blog");
    fake.fail_after("/stream/contents/", 1, Kind::Status(500), 1);
    assert!(run(&output, fake, Arc::new(Publisher::default())).is_err());
    assert!(!is_sandbox(&output));
    let store = Store::open(&output.join("inoink.db"), "local").unwrap();
    assert!(store.has_article(ArticleId(1)).unwrap());
    assert!(!store.download_state(ArticleId(1)).unwrap().readable_offline());
}

struct FailedDiscussion {
    calls: Mutex<Vec<String>>,
}

impl Fetcher for FailedDiscussion {
    fn get(&self, url: &Url, _: usize) -> Result<Fetched> {
        self.calls.lock().unwrap().push(url.to_string());
        Err(Error::Retryable(format!("private-fetch-error: {url}")))
    }
}

#[test]
fn failed_discussion_snapshot_does_not_log_private_urls() {
    const CHILD: &str = "INOINK_SNAPSHOT_STDERR_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let dir = tempdir().unwrap();
        let fake = Fake::new();
        fake.art(1, 1_700_000_000_000_000, "feed/blog");
        fake.art(2, 1_700_000_000_000_001, "feed/blog");
        fake.with(|s| {
            s.arts.get_mut(&1).unwrap().html = "<p>Body</p><a href='https://news.ycombinator.com/item?id=4242'>Comments</a>".into();
            s.arts.get_mut(&2).unwrap().html = "<p>Body</p><a href='https://lobste.rs/s/abc123/private_discussion'>Comments</a>".into();
        });
        let publisher = Arc::new(FailedDiscussion { calls: Mutex::new(Vec::new()) });
        assert_eq!(run(&dir.path().join("data"), fake, publisher.clone()).unwrap().articles, 2);
        let calls = publisher.calls.lock().unwrap();
        assert!(calls.iter().any(|url| url.contains("hn.algolia.com")));
        assert!(calls.iter().any(|url| url.contains("lobste.rs")));
        return;
    }

    // A subprocess captures only this test's stderr, without redirecting the test runner's stderr.
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "failed_discussion_snapshot_does_not_log_private_urls", "--nocapture"])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "child failed: {}", String::from_utf8_lossy(&output.stderr));
    let stderr = String::from_utf8(output.stderr).unwrap();
    for private in ["https://", "news.ycombinator.com", "lobste.rs", "private-fetch-error", "blog.example"] {
        assert!(!stderr.contains(private), "snapshot leaked {private}: {stderr}");
    }
}

#[test]
fn marker_existence_alone_is_offline_only() {
    let dir = tempdir().unwrap();
    assert!(!is_sandbox(dir.path()));
    let marker = dir.path().join(SANDBOX_MARKER);
    fs::write(&marker, "malformed JSON").unwrap();
    assert!(is_sandbox(dir.path()));
    fs::remove_file(&marker).unwrap();
    fs::create_dir(&marker).unwrap();
    assert!(is_sandbox(dir.path()));
    fs::remove_dir(&marker).unwrap();
    symlink(dir.path().join("missing"), marker).unwrap();
    assert!(is_sandbox(dir.path()));
}
