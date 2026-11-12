mod common;
use common::*;
use inoink::api::*;
use inoink::cache::{Cache, CacheConfig};
use inoink::content::images::{Fetched, Fetcher};
use inoink::error::{Error, Result};
use inoink::share::*;
use inoink::store::Store;
use inoink::sync::*;
use std::sync::{Arc, Mutex};
use url::Url;

fn cfg() -> ShareConfig {
    ShareConfig { karakeep: Some(KarakeepConfig { url: "https://keep.example/".into(), api_key: "secret-key".into() }) }
}

#[test]
fn shares_queue_lifecycle_and_dedupe() {
    let mut s = Store::open_in_memory("acct").unwrap();
    let a = s.enqueue_share(KARAKEEP, "https://a.example/x", "A").unwrap();
    let again = s.enqueue_share(KARAKEEP, "https://a.example/x", "A").unwrap();
    assert_eq!(a, again, "the same waiting link is kept once");
    let b = s.enqueue_share(KARAKEEP, "https://b.example/y", "").unwrap();
    assert_eq!(s.share_counts().unwrap(), (2, 0));
    s.share_retry(a, "offline").unwrap();
    let rows = s.pending_shares(10).unwrap();
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![a, b]);
    assert_eq!(rows[0].tries, 1);
    s.share_sent(a).unwrap();
    s.share_failed(b, "refused").unwrap();
    assert_eq!(s.share_counts().unwrap(), (0, 1));
    assert!(s.pending_shares(10).unwrap().is_empty());
}

#[test]
fn karakeep_request_shape() {
    let mock = Mock::new();
    mock.push(Ok(resp(201, "{}")));
    let sharer = HttpSharer::new(cfg(), mock.clone());
    assert_eq!(sharer.share(KARAKEEP, "https://a.example/x", "Hello"), ShareOutcome::Sent);
    let r = mock.last();
    assert_eq!(r.url, "https://keep.example/api/v1/bookmarks");
    assert!(r.headers.iter().any(|(k, v)| k == "Authorization" && v == "Bearer secret-key"));
    match r.body {
        Some(Body::Json(j)) => {
            let v: serde_json::Value = serde_json::from_str(&j).unwrap();
            assert_eq!(v["type"], "link");
            assert_eq!(v["url"], "https://a.example/x");
            assert_eq!(v["title"], "Hello");
        }
        _ => panic!("a JSON body"),
    }
}

#[test]
fn karakeep_status_mapping() {
    let go = |reply: common::Reply| {
        let mock = Mock::new();
        mock.push(reply);
        HttpSharer::new(cfg(), mock).share(KARAKEEP, "https://a.example/x", "")
    };
    assert_eq!(go(Ok(resp(201, ""))), ShareOutcome::Sent);
    assert_eq!(go(Ok(resp(200, ""))), ShareOutcome::Sent, "already saved counts as sent");
    assert!(matches!(go(Ok(resp(401, ""))), ShareOutcome::Failed(m) if m.contains("API key")));
    assert!(matches!(go(Ok(resp(422, ""))), ShareOutcome::Failed(_)));
    assert!(matches!(go(Ok(resp(500, ""))), ShareOutcome::Retry(_)));
    assert!(matches!(go(Err(Error::Offline)), ShareOutcome::Retry(_)));
    assert!(matches!(HttpSharer::new(ShareConfig::default(), Mock::new()).share(KARAKEEP, "https://a", ""), ShareOutcome::Failed(_)));
}

#[test]
fn share_config_round_trip_encrypted() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("provision")).unwrap();
    let recipient = inoink::credentials::ensure_device_key(dir.path()).unwrap();
    let enc = encrypt_for(&recipient, &cfg()).unwrap();
    assert!(!String::from_utf8_lossy(&enc).contains("secret-key"));
    write_transfer(dir.path(), &enc).unwrap();
    assert!(import_provisioned(dir.path()).unwrap());
    assert_eq!(load_config(dir.path()).unwrap(), cfg());
    assert!(!dir.path().join("provision/share.age").exists(), "the transfer file is deleted");
    assert!(!import_provisioned(dir.path()).unwrap());
    let shown = format!("{:?}", cfg());
    assert!(!shown.contains("secret-key"), "the key never shows in debug output");
}

struct Scripted(Mutex<Vec<ShareOutcome>>, Mutex<Vec<String>>);
impl Sharer for Scripted {
    fn share(&self, _t: &str, url: &str, _title: &str) -> ShareOutcome {
        self.1.lock().unwrap().push(url.to_string());
        self.0.lock().unwrap().remove(0)
    }
}

struct NoFetch;
impl Fetcher for NoFetch {
    fn get(&self, _u: &Url, _m: usize) -> Result<Fetched> {
        Ok(Fetched { status: 404, ..Default::default() })
    }
}

#[test]
fn flush_shares_stops_on_retry_and_reports() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FixedClock::new(1_800_000_000);
    let tokens = Arc::new(MemTokens::default());
    let acfg = AppConfig::new("cid", "secret", "http://localhost/cb");
    let api = ApiClient::new(acfg, Mock::new(), clock.clone(), tokens).unwrap();
    let mut store = Store::open(&dir.path().join("db.sqlite"), "42").unwrap();
    for u in ["https://1.example", "https://2.example", "https://3.example", "https://4.example"] {
        store.enqueue_share(KARAKEEP, u, u).unwrap();
    }
    let cache = Cache::new(&dir.path().join("cache"), "42", CacheConfig::default());
    let sharer = Arc::new(Scripted(
        Mutex::new(vec![
            ShareOutcome::Sent,
            ShareOutcome::Failed("nope".into()),
            ShareOutcome::Retry("offline".into()),
            ShareOutcome::Sent,
        ]),
        Mutex::new(vec![]),
    ));
    let mut engine = SyncEngine::new(store, api, cache, Arc::new(NoFetch), clock, SyncConfig::default()).with_sharer(sharer.clone());
    let mut rep = CycleReport::default();
    engine.flush_shares(&mut rep).unwrap();
    assert_eq!(sharer.1.lock().unwrap().len(), 3, "the pass stops at the transient failure");
    assert_eq!(rep.shares.len(), 2);
    assert_eq!(rep.shares[0].error, None);
    assert_eq!(rep.shares[1].error.as_deref(), Some("nope"));
    assert_eq!(engine.store.share_counts().unwrap(), (2, 1), "two still waiting, one failed for good");
}
