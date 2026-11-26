mod common;
use common::*;
use inoink::accounts::RELAY_REDIRECT_URI;
use inoink::setup_server::*;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Env {
    server: SetupServer,
    dir: tempfile::TempDir,
    mock: Arc<Mock>,
    changes: Arc<Mutex<Vec<String>>>,
}

fn start(lifetime: Duration) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let mock = Mock::new();
    let changes = Arc::new(Mutex::new(Vec::new()));
    let c2 = changes.clone();
    let server = SetupServer::start(SetupOptions {
        data_dir: dir.path().to_path_buf(),
        host: Ipv4Addr::LOCALHOST,
        bind: Ipv4Addr::LOCALHOST,
        transport: mock.clone(),
        clock: FixedClock::new(10),
        lifetime,
        token: Some("t0k".into()),
        on_change: Box::new(move |m| c2.lock().unwrap().push(m)),
    })
    .unwrap();
    Env { server, dir, mock, changes }
}

fn http(port: u16, raw: &str) -> (u16, String, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.write_all(raw.as_bytes()).unwrap();
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let text = String::from_utf8_lossy(&out).to_string();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, head.to_string(), body.to_string())
}

fn get(port: u16, path: &str) -> (u16, String, String) {
    http(port, &format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n"))
}

fn post(port: u16, path: &str, body: &str) -> (u16, String, String) {
    http(port, &format!("POST {path} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{body}", body.len()))
}

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn state_of(body: &str) -> String {
    let i = body.find("state=").unwrap() + 6;
    let rest = &body[i..];
    rest[..rest.find(['&', '"']).unwrap()].to_string()
}

fn token_replies(mock: &Mock) {
    mock.push(Ok(resp(200, r#"{"access_token":"a","refresh_token":"r","expires_in":3600,"scope":"read write"}"#)));
    mock.push(Ok(resp(200, r#"{"userId":"1001","userName":"Ann","userEmail":"a@x"}"#)));
}

#[test]
fn wrong_token_is_404() {
    let e = start(Duration::from_secs(30));
    assert_eq!(get(e.server.port(), "/nope").0, 404);
    assert_eq!(get(e.server.port(), "/").0, 404);
    assert_eq!(get(e.server.port(), "/t0kx").0, 404);
}

#[test]
fn page_lists_services_without_secrets() {
    let e = start(Duration::from_secs(30));
    inoink::share::set_karakeep(
        e.dir.path(),
        Some(inoink::share::KarakeepConfig { url: "https://keep.example".into(), api_key: "SECRETKEY".into() }),
    )
    .unwrap();
    let (code, head, body) = get(e.server.port(), "/t0k");
    assert_eq!(code, 200);
    assert!(head.contains("Referrer-Policy: no-referrer"));
    assert!(body.contains("Inoreader") && body.contains("Karakeep") && body.contains("name=\"client_secret\""));
    assert!(!body.contains("SECRETKEY"));
    assert!(body.contains("keep.example"));
}

#[test]
fn karakeep_saved_when_accepted() {
    let e = start(Duration::from_secs(30));
    e.mock.push(Ok(resp(200, "{}")));
    let (code, _, body) = post(e.server.port(), "/t0k/karakeep", &format!("url={}&api_key=abc", enc("https://keep.example/")));
    assert_eq!(code, 200);
    assert!(body.contains("Karakeep is set up"));
    assert_eq!(inoink::share::load_config(e.dir.path()).unwrap().karakeep.unwrap().url, "https://keep.example");
    assert_eq!(e.changes.lock().unwrap().as_slice(), ["Karakeep is set up"]);
}

#[test]
fn karakeep_refused_saves_nothing() {
    let e = start(Duration::from_secs(30));
    e.mock.push(Ok(resp(401, "")));
    let (_, _, body) = post(e.server.port(), "/t0k/karakeep", &format!("url={}&api_key=abc", enc("https://keep.example")));
    assert!(body.contains("refused"));
    assert!(inoink::share::load_config(e.dir.path()).unwrap().karakeep.is_none());
    assert!(e.changes.lock().unwrap().is_empty());
}

#[test]
fn inoreader_relay_flow() {
    let e = start(Duration::from_secs(30));
    let port = e.server.port();
    let form = format!("client_id=id&client_secret=sec&redirect_uri={}", enc(RELAY_REDIRECT_URI));
    let (_, _, body) = post(port, "/t0k/inoreader", &form);
    let state = state_of(&body);
    assert!(state.ends_with(&format!("_127.0.0.1_{port}")), "{state}");
    token_replies(&e.mock);
    let (code, _, _) = get(port, &format!("/callback?code=c&state={state}"));
    assert_eq!(code, 200);
    let c = inoink::credentials::FileCredentials::new(e.dir.path()).load_credentials().unwrap().unwrap();
    assert_eq!(c.user_name, "Ann");
    assert_eq!(e.changes.lock().unwrap().as_slice(), ["Signed in to Inoreader as Ann"]);
}

#[test]
fn inoreader_paste_flow_with_custom_redirect() {
    let e = start(Duration::from_secs(30));
    let port = e.server.port();
    let form = format!("client_id=id&client_secret=sec&redirect_uri={}", enc("http://localhost:8080/callback"));
    let (_, _, body) = post(port, "/t0k/inoreader", &form);
    let state = state_of(&body);
    assert_eq!(state.len(), 32);
    token_replies(&e.mock);
    let addr = format!("http://localhost:8080/callback?code=c&state={state}");
    let (code, _, body) = post(port, "/t0k/inoreader/paste", &format!("address={}", enc(&addr)));
    assert_eq!(code, 200);
    assert!(body.contains("Signed in as Ann"));
}

#[test]
fn callback_with_wrong_state_fails() {
    let e = start(Duration::from_secs(30));
    let port = e.server.port();
    let form = format!("client_id=id&client_secret=sec&redirect_uri={}", enc(RELAY_REDIRECT_URI));
    post(port, "/t0k/inoreader", &form);
    let (_, _, body) = get(port, "/callback?code=c&state=wrong");
    assert!(body.contains("Sign-in failed"));
    assert!(!e.dir.path().join("credentials.json").exists());
    let (code, _, _) = get(port, "/callback?code=c&state=wrong");
    assert_eq!(code, 400, "the pending sign-in is gone");
}

#[test]
fn oversized_body_rejected() {
    let e = start(Duration::from_secs(30));
    let (code, _, _) = http(e.server.port(), "POST /t0k/karakeep HTTP/1.1\r\nHost: x\r\nContent-Length: 100000\r\n\r\n");
    assert_eq!(code, 413);
}

#[test]
fn stops_after_lifetime() {
    let e = start(Duration::from_millis(300));
    let port = e.server.port();
    std::thread::sleep(Duration::from_secs(1));
    let r = TcpStream::connect_timeout(&format!("127.0.0.1:{port}").parse().unwrap(), Duration::from_millis(500));
    if let Ok(mut s) = r {
        // The listener may linger in the backlog; it must not answer.
        s.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
        let _ = s.write_all(b"GET /t0k HTTP/1.1\r\n\r\n");
        let mut b = Vec::new();
        let _ = s.read_to_end(&mut b);
        assert!(b.is_empty());
    }
}
