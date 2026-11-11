#![allow(dead_code)]
use inoink::api::*;
use inoink::error::{Error, Result};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

pub type Reply = Result<HttpResponse>;

/// Scripted transport: replies come from a queue; every request is recorded.
#[derive(Default)]
pub struct Mock {
    pub replies: Mutex<VecDeque<Reply>>,
    pub requests: Mutex<Vec<HttpRequest>>,
    /// Used when the queue is empty.
    pub fallback: Mutex<Option<Box<dyn Fn(&HttpRequest) -> Reply + Send>>>,
}

impl Mock {
    pub fn new() -> Arc<Mock> {
        Arc::new(Mock::default())
    }
    pub fn push(&self, r: Reply) {
        self.replies.lock().unwrap().push_back(r);
    }
    pub fn push_json(&self, v: serde_json::Value) {
        self.push(Ok(resp(200, &v.to_string())));
    }
    pub fn push_ok(&self) {
        self.push(Ok(resp(200, "OK")));
    }
    pub fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
    pub fn last(&self) -> HttpRequest {
        self.requests.lock().unwrap().last().cloned().expect("no request")
    }
    pub fn all(&self) -> Vec<HttpRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Transport for Mock {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse> {
        self.requests.lock().unwrap().push(req.clone());
        if let Some(r) = self.replies.lock().unwrap().pop_front() {
            return r;
        }
        if let Some(f) = self.fallback.lock().unwrap().as_ref() {
            return f(req);
        }
        Err(Error::Offline)
    }
}

pub fn resp(status: u16, body: &str) -> HttpResponse {
    HttpResponse { status, headers: vec![], body: body.as_bytes().to_vec() }
}

pub fn resp_h(status: u16, body: &str, headers: &[(&str, &str)]) -> HttpResponse {
    HttpResponse {
        status,
        headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        body: body.as_bytes().to_vec(),
    }
}

pub struct FixedClock(pub Mutex<i64>);
impl FixedClock {
    pub fn new(t: i64) -> Arc<FixedClock> {
        Arc::new(FixedClock(Mutex::new(t)))
    }
    pub fn advance(&self, s: i64) {
        *self.0.lock().unwrap() += s;
    }
}
impl Clock for FixedClock {
    fn now(&self) -> i64 {
        *self.0.lock().unwrap()
    }
}

#[derive(Default)]
pub struct MemTokens {
    pub tokens: Mutex<Option<Tokens>>,
    pub saves: Mutex<Vec<Tokens>>,
}
impl TokenStore for MemTokens {
    fn load(&self) -> Result<Option<Tokens>> {
        Ok(self.tokens.lock().unwrap().clone())
    }
    fn save(&self, t: &Tokens) -> Result<()> {
        *self.tokens.lock().unwrap() = Some(t.clone());
        self.saves.lock().unwrap().push(t.clone());
        Ok(())
    }
}

pub fn tokens(expires_at: i64) -> Tokens {
    Tokens { access_token: "acc".into(), refresh_token: "ref".into(), expires_at, scope: "read write".into() }
}

pub struct Harness {
    pub mock: Arc<Mock>,
    pub clock: Arc<FixedClock>,
    pub tokens: Arc<MemTokens>,
    pub client: ApiClient,
}

pub fn harness() -> Harness {
    let mock = Mock::new();
    let clock = FixedClock::new(1_000_000);
    let store = Arc::new(MemTokens::default());
    *store.tokens.lock().unwrap() = Some(tokens(1_000_000 + 3600));
    let mut cfg = AppConfig::new("cid", "secret", "http://localhost:8080/cb");
    cfg.base_url = "https://example.test/reader/api/0".into();
    cfg.token_url = "https://example.test/oauth2/token".into();
    let client = ApiClient::new(cfg, mock.clone(), clock.clone(), store.clone()).unwrap();
    Harness { mock, clock, tokens: store, client }
}

pub fn form(req: &HttpRequest) -> Vec<(String, String)> {
    match &req.body {
        Some(Body::Form(f)) => f.clone(),
        other => panic!("expected form body, got {other:?}"),
    }
}

pub fn form_get(req: &HttpRequest, key: &str) -> Vec<String> {
    form(req).into_iter().filter(|(k, _)| k == key).map(|(_, v)| v).collect()
}

pub fn query(url: &str) -> Vec<(String, String)> {
    url::Url::parse(url).unwrap().query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect()
}

pub fn q1(url: &str, key: &str) -> Option<String> {
    query(url).into_iter().find(|(k, _)| k == key).map(|(_, v)| v)
}
