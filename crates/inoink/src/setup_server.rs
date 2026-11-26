//! A small web page the Kobo serves on the Wi-Fi, so accounts can be set up from a phone: its
//! address is shown as a QR code. It lives for a limited time and its address contains a random
//! token. Plain HTTP on the local network: the page says so.
//!
//! Inoreader sign-in goes through the project's redirect page (`accounts::RELAY_REDIRECT_URI`),
//! which hands the result back to this server's `/callback`; the address can also be pasted.

use crate::accounts::{self, Service, RELAY_REDIRECT_URI};
use crate::api::auth::{authorize_url, parse_callback, AppConfig};
use crate::api::{Clock, Transport};
use crate::credentials::FileCredentials;
use crate::error::{Error, Result};
use crate::share::{self, KarakeepCheck, KarakeepConfig};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 8 * 1024;
const PENDING_LIFETIME: Duration = Duration::from_secs(600);

pub struct SetupOptions {
    pub data_dir: PathBuf,
    /// The address shown in the URL and used in the redirect state.
    pub host: Ipv4Addr,
    /// What to listen on: 0.0.0.0 on the Kobo, 127.0.0.1 in tests.
    pub bind: Ipv4Addr,
    pub transport: Arc<dyn Transport>,
    pub clock: Arc<dyn Clock>,
    pub lifetime: Duration,
    /// Tests only; otherwise random.
    pub token: Option<String>,
    /// Called with a message after an account was saved.
    pub on_change: Box<dyn Fn(String) + Send>,
}

pub struct SetupServer {
    url: String,
    port: u16,
    stop: Arc<AtomicBool>,
    _join: Option<JoinHandle<()>>,
}

impl SetupServer {
    pub fn start(o: SetupOptions) -> Result<SetupServer> {
        let token = match &o.token {
            Some(t) => t.clone(),
            None => crate::util::random_hex(16)?,
        };
        let listener = TcpListener::bind((o.bind, 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let url = format!("http://{}:{}/{}", o.host, port, token);
        let stop = Arc::new(AtomicBool::new(false));
        let mut st = State { o, token, port, pending: None };
        let stop2 = stop.clone();
        let join = std::thread::Builder::new()
            .name("inoink-setup".into())
            .spawn(move || {
                let end = Instant::now() + st.o.lifetime;
                while !stop2.load(Ordering::Relaxed) && Instant::now() < end {
                    match listener.accept() {
                        Ok((s, _)) => {
                            let _ = handle(&mut st, s);
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
                        Err(_) => std::thread::sleep(Duration::from_millis(200)),
                    }
                }
            })
            .map_err(|e| Error::Io(e.to_string()))?;
        Ok(SetupServer { url, port, stop, _join: Some(join) })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for SetupServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// This device's address on the local network, when it has one.
pub fn local_ipv4() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    match s.local_addr().ok()? {
        SocketAddr::V4(a) if !a.ip().is_unspecified() && !a.ip().is_loopback() => Some(*a.ip()),
        _ => None,
    }
}

struct Pending {
    cfg: AppConfig,
    state: String,
    at: Instant,
}

struct State {
    o: SetupOptions,
    token: String,
    port: u16,
    pending: Option<Pending>,
}

struct Request {
    method: String,
    path: String,
    query: String,
    body: Vec<u8>,
}

fn read_request(s: &mut TcpStream) -> std::result::Result<Request, u16> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > MAX_HEAD {
            return Err(431);
        }
        match s.read(&mut chunk) {
            Ok(0) | Err(_) => return Err(400),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.lines();
    let first = lines.next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("");
    let (path, query) = target.split_once('?').map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or((target.to_string(), String::new()));
    let len = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    if len > MAX_BODY {
        return Err(413);
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        match s.read(&mut chunk) {
            Ok(0) | Err(_) => return Err(400),
            Ok(n) => body.extend_from_slice(&chunk[..n]),
        }
    }
    body.truncate(len);
    Ok(Request { method, path, query, body })
}

fn respond(s: &mut TcpStream, status: u16, body: &str) {
    let text = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        413 => "Payload Too Large",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {text}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nX-Frame-Options: DENY\r\n\r\n",
        body.len()
    );
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body.as_bytes());
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

fn shell(title: &str, inner: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><meta name=referrer content=no-referrer><title>{t}</title><style>body{{font:17px/1.4 system-ui,sans-serif;max-width:34rem;margin:1rem auto;padding:0 1rem}}input{{width:100%;box-sizing:border-box;padding:.5rem;margin:.2rem 0 .6rem;font-size:1rem}}button,.btn{{padding:.6rem 1rem;font-size:1rem}}small{{color:#555}}.err{{color:#a00}}.note{{background:#f4f0e0;padding:.6rem}}</style></head><body><h1>{t}</h1>{inner}</body></html>",
        t = escape(title)
    )
}

fn page(st: &State, error: Option<&str>) -> String {
    let dir = &st.o.data_dir;
    let creds = FileCredentials::new(dir).load_credentials().ok().flatten();
    let karakeep = share::load_config(dir).ok().and_then(|c| c.karakeep);
    let mut h = String::from("<p class=note>This page comes from your Kobo over your Wi-Fi, without encryption. Use it on a network you trust. It stops working 15 minutes after it was opened on the Kobo.</p>");
    if let Some(e) = error {
        h.push_str(&format!("<p class=err>{}</p>", escape(e)));
    }
    for s in Service::ALL {
        h.push_str(&format!("<h2>{}</h2><p>{}</p>", s.name(), escape(&accounts::describe(dir, s).unwrap_or_else(|| "Not set up".into()))));
        h.push_str(&format!("<form method=post action=\"/{}/{}\">", escape(&st.token), s.id()));
        for f in s.fields() {
            let prefill = match (s, f.key) {
                (Service::Inoreader, "client_id") => creds.as_ref().map(|c| c.client_id.clone()).unwrap_or_default(),
                (Service::Inoreader, "redirect_uri") => {
                    creds.as_ref().map(|c| c.redirect_uri.clone()).unwrap_or_else(|| RELAY_REDIRECT_URI.to_string())
                }
                (Service::Karakeep, "url") => karakeep.as_ref().map(|k| k.url.clone()).unwrap_or_default(),
                _ => String::new(),
            };
            h.push_str(&format!(
                "<label>{}<input name=\"{}\" type=\"{}\" value=\"{}\" autocomplete=off autocapitalize=off spellcheck=false></label>",
                escape(f.label),
                f.key,
                if f.secret { "password" } else { "text" },
                escape(&prefill)
            ));
            if !f.hint.is_empty() {
                h.push_str(&format!("<small>{}</small><br>", escape(f.hint)));
            }
        }
        h.push_str(&format!(
            "<button>{}</button></form>",
            if s == Service::Inoreader { "Sign in with Inoreader" } else { "Save Karakeep" }
        ));
    }
    shell("Set up InoInk", &h)
}

fn link_back(st: &State) -> String {
    format!("<p><a href=\"/{}\">Back to the setup page</a></p>", escape(&st.token))
}

fn form(body: &[u8]) -> std::collections::HashMap<String, String> {
    url::form_urlencoded::parse(body).map(|(k, v)| (k.into_owned(), v.into_owned())).collect()
}

fn handle(st: &mut State, mut s: TcpStream) -> std::io::Result<()> {
    s.set_nonblocking(false)?;
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    s.set_write_timeout(Some(Duration::from_secs(10)))?;
    let req = match read_request(&mut s) {
        Ok(r) => r,
        Err(code) => {
            respond(&mut s, code, &shell("Error", "<p>Bad request.</p>"));
            return Ok(());
        }
    };
    if req.method == "GET" && req.path == "/callback" {
        let full = format!("/callback?{}", req.query);
        let (code, body) = finish_sign_in(st, &full, false);
        respond(&mut s, code, &body);
        return Ok(());
    }
    let prefix = format!("/{}", st.token);
    let rest = match req.path.strip_prefix(&prefix) {
        Some(r) if r.is_empty() || r.starts_with('/') => r.trim_end_matches('/').to_string(),
        _ => {
            respond(&mut s, 404, &shell("Not found", "<p>Not found.</p>"));
            return Ok(());
        }
    };
    match (req.method.as_str(), rest.as_str()) {
        ("GET", "") => respond(&mut s, 200, &page(st, None)),
        ("POST", "/karakeep") => {
            let f = form(&req.body);
            let (code, body) =
                save_karakeep(st, f.get("url").map(String::as_str).unwrap_or(""), f.get("api_key").map(String::as_str).unwrap_or(""));
            respond(&mut s, code, &body);
        }
        ("POST", "/inoreader") => {
            let f = form(&req.body);
            let get = |k: &str| f.get(k).map(|v| v.trim().to_string()).unwrap_or_default();
            let (id, secret, redirect) = (get("client_id"), get("client_secret"), get("redirect_uri"));
            if id.is_empty() || secret.is_empty() || redirect.is_empty() {
                respond(&mut s, 200, &page(st, Some("Fill in the App ID, App key and redirect URI")));
            } else {
                let body = begin_sign_in(st, &id, &secret, &redirect);
                respond(&mut s, 200, &body);
            }
        }
        ("POST", "/inoreader/paste") => {
            let f = form(&req.body);
            let (code, body) = finish_sign_in(st, f.get("address").map(|a| a.trim()).unwrap_or(""), true);
            respond(&mut s, code, &body);
        }
        _ => respond(&mut s, 404, &shell("Not found", "<p>Not found.</p>")),
    }
    Ok(())
}

fn save_karakeep(st: &mut State, url: &str, key: &str) -> (u16, String) {
    let url = match share::karakeep_base(url) {
        Ok(u) => u,
        Err(e) => return (200, page(st, Some(&e.to_string()))),
    };
    let key = key.trim();
    if key.is_empty() {
        return (200, page(st, Some("Enter the API key")));
    }
    let k = KarakeepConfig { url, api_key: key.to_string() };
    let note = match share::check_karakeep(&k, st.o.transport.as_ref()) {
        KarakeepCheck::Refused => return (200, page(st, Some("Karakeep refused the API key. Nothing was saved."))),
        KarakeepCheck::Accepted => String::new(),
        KarakeepCheck::Unverified(e) => format!("<p>The key could not be checked ({}); it was saved anyway.</p>", escape(&e)),
    };
    if let Err(e) = share::set_karakeep(&st.o.data_dir, Some(k)) {
        return (200, page(st, Some(&format!("Could not save: {e}"))));
    }
    (st.o.on_change)("Karakeep is set up".into());
    (200, shell("Karakeep", &format!("<p>Karakeep is set up.</p>{note}{}", link_back(st))))
}

fn begin_sign_in(st: &mut State, id: &str, secret: &str, redirect: &str) -> String {
    let nonce = match crate::util::random_hex(16) {
        Ok(n) => n,
        Err(e) => return page(st, Some(&e.to_string())),
    };
    let relay = redirect == RELAY_REDIRECT_URI;
    let state = if relay { accounts::relay_state(&nonce, &st.o.host.to_string(), st.port) } else { nonce };
    let cfg = AppConfig::new(id, secret, redirect);
    let link = authorize_url(&cfg, &state, "read write");
    st.pending = Some(Pending { cfg, state, at: Instant::now() });
    let mut h = format!("<p><a class=btn href=\"{}\" style=\"font-size:1.3rem\">Open Inoreader</a></p>", escape(&link));
    if relay {
        h.push_str("<p>After you approve, this browser comes back to your Kobo on its own.</p>");
    }
    h.push_str(&format!(
        "<p>If it does not, copy the address your browser ends on and paste it here:</p><form method=post action=\"/{}/inoreader/paste\"><input name=address autocomplete=off><button>Finish sign-in</button></form>",
        escape(&st.token)
    ));
    shell("Approve InoInk in Inoreader", &h)
}

fn finish_sign_in(st: &mut State, address: &str, with_link: bool) -> (u16, String) {
    let pending = match st.pending.take() {
        Some(p) if p.at.elapsed() < PENDING_LIFETIME => p,
        _ => {
            return (
                400,
                shell(
                    "Sign-in",
                    &format!(
                        "<p>No sign-in is waiting. Start again on the setup page.</p>{}",
                        if with_link { link_back(st) } else { String::new() }
                    ),
                ),
            )
        }
    };
    let fail = |st: &State, why: String| {
        (
            200,
            shell(
                "Sign-in",
                &format!(
                    "<p class=err>Sign-in failed: {}. Start again on the setup page.</p>{}",
                    escape(&why),
                    if with_link { link_back(st) } else { String::new() }
                ),
            ),
        )
    };
    let code = match parse_callback(address, &pending.state) {
        Ok(c) => c,
        Err(e) => return fail(st, e.to_string()),
    };
    let creds = match accounts::sign_in_inoreader(&pending.cfg, st.o.transport.clone(), st.o.clock.clone(), &code) {
        Ok(c) => c,
        Err(e) => return fail(st, e.to_string()),
    };
    if let Err(e) = FileCredentials::new(&st.o.data_dir).save_credentials(&creds) {
        return fail(st, e.to_string());
    }
    let who = if creds.user_name.is_empty() { creds.user_id.clone() } else { creds.user_name.clone() };
    (st.o.on_change)(format!("Signed in to Inoreader as {who}"));
    let more = if with_link { format!(" or set up Karakeep.{}", link_back(st)) } else { ".".into() };
    (200, shell("Signed in", &format!("<p>Signed in as {}. InoInk starts syncing. You can close this page{more}</p>", escape(&who))))
}
