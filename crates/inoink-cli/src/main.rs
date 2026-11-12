//! Computer-side helper for InoInk: authorize with Inoreader, provision a Kobo, build fixture
//! data and check a cache. Secrets are never taken from command-line arguments.

use inoink::api::auth::{authorize_url, exchange_code, parse_callback, AppConfig};
use inoink::api::{ApiClient, Clock, SystemClock, UreqTransport};
use inoink::cache::{Cache, CacheConfig, Control};
use inoink::content::images::{Fetched, Fetcher};
use inoink::credentials::{encrypt_for, read_device_recipient, write_transfer, Credentials};
use inoink::error::{Error, Result};
use inoink::model::*;
use inoink::store::{ArticleRecord, Store, Subscription};
use std::io::{BufRead, Cursor, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const USAGE: &str = "inoink-cli <command>

  authorize --client-id ID --redirect-uri URI [--scope 'read write'] (--device MOUNT | --out FILE)
      Sign in with Inoreader on this computer. The client secret is read from the
      INOINK_CLIENT_SECRET environment variable or a hidden prompt. With --device the encrypted
      credentials are written to the mounted Kobo; with --out they are written to FILE
      (plaintext: delete it after provisioning).
  provision --credentials FILE --device MOUNT
      Encrypt previously authorized credentials for the Kobo's key.
  fixtures --output DIR
      Build deterministic offline sample data (articles, images, tables, code).
  verify-cache --data DIR [--account ID]
      Check published content versions and images.
  acceptance --live --credentials FILE [--yes]
      Read-only checks against a real account (shows what it will request first).
";

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn data_dir_of(mount: &Path) -> PathBuf {
    mount.join(".adds").join("inoink").join("data")
}

fn random_hex(n: usize) -> Result<String> {
    let mut buf = vec![0u8; n];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

fn read_secret() -> Result<String> {
    if let Ok(s) = std::env::var("INOINK_CLIENT_SECRET") {
        if !s.trim().is_empty() {
            eprintln!("Using the client secret from INOINK_CLIENT_SECRET.");
            return Ok(s.trim().to_string());
        }
    }
    for _ in 0..3 {
        eprint!("Paste the app key (client secret), then press Enter. Nothing is shown while you type: ");
        let _ = std::process::Command::new("stty").arg("-echo").stdin(std::process::Stdio::inherit()).status();
        let mut line = String::new();
        let r = std::io::stdin().lock().read_line(&mut line);
        let _ = std::process::Command::new("stty").arg("echo").stdin(std::process::Stdio::inherit()).status();
        eprintln!();
        r?;
        let mut s = line.trim().to_string();
        // Terminals sometimes deliver a paste twice; a key repeated back to back is collapsed.
        let n = s.chars().count();
        if n.is_multiple_of(2) && n > 0 {
            let half: String = s.chars().take(n / 2).collect();
            if s == format!("{half}{half}") {
                eprintln!("The key arrived twice; using it once.");
                s = half;
            }
        }
        if s.is_empty() {
            eprintln!("Nothing was received. Try again.");
            continue;
        }
        // Confirm receipt without revealing the value.
        eprintln!(
            "Received a {}-character key ending in ...{}.",
            s.chars().count(),
            s.chars().rev().take(2).collect::<String>().chars().rev().collect::<String>()
        );
        return Ok(s);
    }
    Err(Error::InvalidRequest("no client secret was entered".into()))
}

/// Waits for the browser redirect on a loopback address and, at the same time, accepts the
/// redirected address pasted into the terminal. Whichever arrives first wins.
fn receive_callback(redirect_uri: &str) -> Result<String> {
    let url = url::Url::parse(redirect_uri).map_err(|_| Error::InvalidRequest("bad redirect URI".into()))?;
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let loopback = matches!(url.host_str(), Some("localhost") | Some("127.0.0.1")) && url.port().is_some();
    if loopback {
        match TcpListener::bind(("127.0.0.1", url.port().unwrap())) {
            Ok(listener) => {
                let tx = tx.clone();
                let base = format!("{}://{}", url.scheme(), url.host_str().unwrap_or("localhost"));
                std::thread::spawn(move || {
                    for stream in listener.incoming() {
                        let Ok(mut stream) = stream else { continue };
                        let mut reader = std::io::BufReader::new(match stream.try_clone() {
                            Ok(s) => s,
                            Err(_) => continue,
                        });
                        let mut first = String::new();
                        if reader.read_line(&mut first).is_err() {
                            continue;
                        }
                        let path = first.split_whitespace().nth(1).unwrap_or("").to_string();
                        if !path.contains("code=") && !path.contains("error=") {
                            // Favicon and other stray requests.
                            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n");
                            continue;
                        }
                        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nInoInk is authorized. You can close this tab.\r\n");
                        let _ = tx.send(format!("{base}{path}"));
                        break;
                    }
                });
            }
            Err(e) => eprintln!("(Could not listen on the redirect port: {e}. Paste the address instead.)"),
        }
    }
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        loop {
            let mut line = String::new();
            match stdin.lock().read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if line.contains("code=") || line.contains("error=") {
                let _ = tx.send(line.to_string());
                break;
            }
            if line.contains("/oauth2/auth") {
                eprintln!("That is the approval page itself. Open it in your browser, approve, then paste the address the browser ends on (it starts with the redirect URI and contains code=).");
            } else {
                eprintln!("That address has no code= in it. Paste the address the browser shows after you approve.");
            }
        }
    });
    eprintln!("Waiting for the browser to return here. If it does not, paste the address it ended on and press Enter:");
    rx.recv().map_err(|_| Error::InvalidRequest("no authorization was received".into()))
}

fn authorize(args: &[String]) -> Result<()> {
    let client_id = arg(args, "--client-id").ok_or_else(|| Error::InvalidRequest("--client-id is required".into()))?;
    let redirect = arg(args, "--redirect-uri").ok_or_else(|| Error::InvalidRequest("--redirect-uri is required".into()))?;
    let scope = arg(args, "--scope").unwrap_or_else(|| "read write".into());
    let device = arg(args, "--device").map(PathBuf::from);
    let out = arg(args, "--out").map(PathBuf::from);
    if device.is_none() && out.is_none() {
        return Err(Error::InvalidRequest("give --device MOUNT or --out FILE".into()));
    }
    let recipient = match &device {
        Some(m) => Some(read_device_recipient(&data_dir_of(m))?),
        None => None,
    };
    let secret = read_secret()?;
    let cfg = AppConfig::new(&client_id, &secret, &redirect);
    let state = random_hex(16)?;
    let consent = authorize_url(&cfg, &state, &scope);
    println!("Approve access in the browser. If it does not open, open this address:\n\n  {consent}\n");
    let _ = std::process::Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" }).arg(&consent).status();
    let callback = receive_callback(&redirect)?;
    let code = parse_callback(&callback, &state)?;
    let transport = UreqTransport::new();
    let tokens = exchange_code(&cfg, &transport, &SystemClock, &code)?;
    // Learn the account id with a throwaway in-memory token store.
    struct Mem(std::sync::Mutex<Option<inoink::api::Tokens>>);
    impl inoink::api::TokenStore for Mem {
        fn load(&self) -> Result<Option<inoink::api::Tokens>> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn save(&self, t: &inoink::api::Tokens) -> Result<()> {
            *self.0.lock().unwrap() = Some(t.clone());
            Ok(())
        }
    }
    let api = ApiClient::new(
        cfg.clone(),
        Arc::new(UreqTransport::new()),
        Arc::new(SystemClock),
        Arc::new(Mem(std::sync::Mutex::new(Some(tokens.clone())))),
    )?;
    let info = api.user_info()?;
    let mut creds = Credentials::new(&cfg, &tokens);
    creds.user_id = info.user_id.clone();
    println!("Signed in as {}.", info.user_name);
    if let Some(r) = recipient {
        let enc = encrypt_for(&r, &creds)?;
        let dir = data_dir_of(device.as_ref().unwrap());
        write_transfer(&dir, &enc)?;
        println!("Credentials were encrypted for this Kobo and written to {}.", dir.join("provision/credentials.age").display());
        println!("Eject the Kobo and open InoInk; it imports and deletes the file.");
    }
    if let Some(o) = out {
        if let Some(dir) = o.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&o)?;
            f.write_all(&serde_json::to_vec_pretty(&creds)?)?;
        }
        println!("Wrote {} (plaintext credentials; delete it after provisioning).", o.display());
    }
    Ok(())
}

fn provision(args: &[String]) -> Result<()> {
    let file = arg(args, "--credentials").ok_or_else(|| Error::InvalidRequest("--credentials is required".into()))?;
    let device = arg(args, "--device").ok_or_else(|| Error::InvalidRequest("--device is required".into()))?;
    let creds: Credentials = serde_json::from_slice(&std::fs::read(&file)?).map_err(|_| Error::Data("not a credentials file".into()))?;
    let dir = data_dir_of(Path::new(&device));
    let enc = encrypt_for(&read_device_recipient(&dir)?, &creds)?;
    write_transfer(&dir, &enc)?;
    println!("Encrypted credentials written for the Kobo at {}.", dir.display());
    Ok(())
}

// ---- fixtures ---------------------------------------------------------------------------------

fn png_gradient(w: u32, h: u32, seed: u8) -> Vec<u8> {
    let mut img = image::RgbImage::new(w, h);
    for (x, y, p) in img.enumerate_pixels_mut() {
        let r = ((x * 255) / w.max(1)) as u8;
        let g = ((y * 255) / h.max(1)) as u8;
        let b = seed.wrapping_mul(37).wrapping_add(((x + y) % 64) as u8 * 3);
        *p = image::Rgb([r, g, b]);
    }
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

struct FixtureImages;
impl Fetcher for FixtureImages {
    fn get(&self, url: &url::Url, _max: usize) -> Result<Fetched> {
        if url.as_str() == "https://blog.example/posts/linked-story" {
            let body: String = (1..=8).map(paragraph).collect();
            let page = format!("<html><body><nav>Menu Menu Menu</nav><article><h1>The linked story</h1>{body}</article></body></html>");
            return Ok(Fetched { status: 200, content_type: Some("text/html; charset=utf-8".into()), location: None, body: page.into_bytes() });
        }
        if url.as_str() == "https://hn.algolia.com/api/v1/items/4242" {
            let json = r#"{"type":"story","title":"A linked story","url":"https://blog.example/posts/linked-story","text":null,"children":[
              {"type":"comment","author":"alice","text":"<p>Great write-up. The part about <i>pagination</i> is what I keep running into.</p>","created_at_i":1799990000,"children":[
                {"type":"comment","author":"bob","text":"<p>Agreed, though I would not call it solved.</p>","created_at_i":1799990600,"children":[
                  {"type":"comment","author":"alice","text":"<p>Fair. Solved enough for e-ink.</p>","created_at_i":1799991200,"children":[]}]}]},
              {"type":"comment","author":null,"text":null,"children":[
                {"type":"comment","author":"carol","text":"<p>Replying under a deleted comment.</p>","created_at_i":1799992000,"children":[]}]}]}"#;
            return Ok(Fetched { status: 200, content_type: Some("application/json".into()), location: None, body: json.as_bytes().to_vec() });
        }
        let name = url.path().rsplit('/').next().unwrap_or("");
        let seed = name.bytes().fold(7u8, |a, b| a.wrapping_mul(31).wrapping_add(b));
        let (w, h) = match name {
            n if n.starts_with("wide") => (1200, 600),
            n if n.starts_with("tall") => (600, 1000),
            n if n.starts_with("missing") => return Ok(Fetched { status: 404, ..Default::default() }),
            _ => (800, 500),
        };
        Ok(Fetched { status: 200, content_type: Some("image/png".into()), location: None, body: png_gradient(w, h, seed) })
    }
}

fn paragraph(i: usize) -> String {
    format!(
        "<p>Paragraph {i}. The quick brown fox jumps over the lazy dog while a long essay keeps unfolding across many pages, \
         with enough words to wrap several lines on a six-inch display and exercise hyphenation, justification and pagination. \
         Caf\u{e9} cr\u{e8}me br\u{fb}l\u{e9}e, na\u{ef}ve fa\u{e7}ades, and \u{201c}typographic quotes\u{201d} are included on purpose.</p>"
    )
}

/// Deterministic sample articles: (title, html).
pub fn fixture_articles() -> Vec<(&'static str, String)> {
    let essay: String = (1..=60).map(paragraph).collect();
    let images = r#"<p>An image-heavy photo essay.</p>
<figure><img src="https://img.example/wide-1.png" alt="Wide landscape"><figcaption>A wide landscape with a long caption that wraps.</figcaption></figure>
<p>Between the pictures there is some text.</p>
<figure><img src="https://img.example/tall-1.png" alt="Tall portrait"><figcaption>A tall portrait.</figcaption></figure>
<img src="https://img.example/small-1.png" alt="Inline image">
<img src="https://img.example/missing-1.png" alt="This one never loads">
<figure><img src="//img.example/wide-2.png" srcset="//img.example/small-2.png 400w, //img.example/wide-2.png 1200w" alt="Responsive"><figcaption>Responsive image.</figcaption></figure>"#
        .to_string();
    let newsletter =
        r#"<div><h2>This week</h2><p>Hello reader, here is your weekly <a href="https://news.example/issue/42">newsletter</a>.</p>
<ul><li>First item with <b>bold</b> and <i>italic</i></li><li>Second item</li><li>Third item</li></ul>
<blockquote><p>A pulled quote that should be indented.</p></blockquote><hr><p><small>Unsubscribe</small></p></div>"#
            .to_string();
    let table = r#"<p>Quarterly numbers.</p><table><caption>Results</caption><thead><tr><th>Quarter</th><th>Revenue</th><th>Notes</th></tr></thead>
<tbody><tr><td>Q1</td><td>1,200</td><td>Steady</td></tr><tr><td>Q2</td><td>1,450</td><td>Up</td></tr><tr><td colspan="2">Total</td><td>2,650</td></tr></tbody></table>"#
        .to_string();
    let code = r#"<p>A code listing:</p><pre><code>fn main() {
    let items = vec![1, 2, 3];
    for (i, x) in items.iter().enumerate() {
        println!("{i}: {x} &amp; a rather long line that has to be wrapped or scrolled because it exceeds the width");
    }
}
</code></pre><p>Inline <code>code</code> too.</p>"#
        .to_string();
    vec![
        ("A long essay", essay),
        ("Photo essay with images", images),
        ("Weekly newsletter", newsletter),
        ("Table of results", table),
        ("Code listing", code),
        // A Hacker News link post: the full text lives on the linked page.
        ("A linked story", "<a href=\"https://news.ycombinator.com/item?id=4242\">Comments</a>".to_string()),
    ]
}

fn fixtures(args: &[String]) -> Result<()> {
    let out = PathBuf::from(arg(args, "--output").ok_or_else(|| Error::InvalidRequest("--output is required".into()))?);
    std::fs::create_dir_all(&out)?;
    let db = out.join("inoink.db");
    let _ = std::fs::remove_file(&db);
    let mut store = Store::open(&db, "local")?;
    let cache = Cache::new(&out.join("cache"), "local", CacheConfig::default());
    let feed = "feed/https://blog.example/rss";
    store.replace_subscriptions(&[Subscription {
        id: feed.into(),
        title: "Example Blog".into(),
        url: "https://blog.example/rss".into(),
        html_url: String::new(),
        sort_id: String::new(),
        first_item_usec: 0,
        folders: vec!["Reading".into()],
    }])?;
    let base_ts = 1_800_000_000i64;
    let ctl = Control::new(Arc::new(AtomicBool::new(false)));
    for (i, (title, html)) in fixture_articles().into_iter().enumerate() {
        let id = 1000 + i as u64;
        let mut cats = vec!["user/1/state/com.google/reading-list".to_string()];
        if i == 0 || i == 1 || i == 5 {
            cats.push("user/1/state/com.google/starred".into());
        }
        let rec = ArticleRecord {
            id,
            title: title.into(),
            author: "Fixture Author".into(),
            origin_stream: if i == 5 { "feed/https://news.ycombinator.com/rss".into() } else { feed.into() },
            origin_title: if i == 5 { "Hacker News".into() } else { "Example Blog".into() },
            canonical_url: if i == 5 { "https://blog.example/posts/linked-story".into() } else { format!("https://blog.example/posts/{id}") },
            published: base_ts - i as i64 * 3600,
            timestamp_usec: (base_ts - i as i64 * 3600) * 1_000_000,
            summary_html: html,
            raw_json: "{}".into(),
            categories: cats,
            excerpt: false,
        };
        store.upsert_article(&rec, 0)?;
        let state = cache.cache_article(&mut store, &FixtureImages, ArticleId(id), &ctl)?;
        println!("{id}: {title} -> {state:?}");
    }
    store.checkpoint()?;
    println!("Fixture data written to {}", out.display());
    Ok(())
}

fn verify_cache(args: &[String]) -> Result<()> {
    let data = PathBuf::from(arg(args, "--data").ok_or_else(|| Error::InvalidRequest("--data is required".into()))?);
    let account = arg(args, "--account").unwrap_or_else(|| "local".into());
    let db = if data.join("inoink.db").exists() { data.join("inoink.db") } else { data.join("db.sqlite") };
    let store = Store::open(&db, &account)?;
    let cache = Cache::new(&data.join("cache"), &account, CacheConfig::default());
    let rep = cache.verify(&store)?;
    println!("Checked {} article(s).", rep.checked);
    for p in &rep.problems {
        println!("PROBLEM: {p}");
    }
    if rep.problems.is_empty() {
        println!("Cache is consistent.");
        Ok(())
    } else {
        Err(Error::Data(format!("{} problem(s)", rep.problems.len())))
    }
}

fn acceptance(args: &[String]) -> Result<()> {
    if !flag(args, "--live") {
        return Err(Error::InvalidRequest("acceptance only runs with --live".into()));
    }
    let file = arg(args, "--credentials").ok_or_else(|| Error::InvalidRequest("--credentials FILE is required".into()))?;
    let creds: Credentials = serde_json::from_slice(&std::fs::read(&file)?).map_err(|_| Error::Data("not a credentials file".into()))?;
    println!("Read-only live checks (each counts against your daily quota):");
    for l in [
        "GET /user-info",
        "GET /subscription/list",
        "GET /tag/list?types=1&counts=1",
        "GET /unread-count",
        "GET /preference/stream/list",
        "GET /stream/items/ids (starred, n=5)",
        "GET /stream/contents (reading-list, n=2, annotations, summaries)",
    ] {
        println!("  - {l}");
    }
    println!("No state-changing requests are made.");
    if !flag(args, "--yes") {
        println!("Re-run with --yes to proceed.");
        return Ok(());
    }
    // Tokens refreshed during the checks are written back to the same file.
    struct PathTokens(PathBuf, std::sync::Mutex<Credentials>);
    impl inoink::api::TokenStore for PathTokens {
        fn load(&self) -> Result<Option<inoink::api::Tokens>> {
            Ok(Some(self.1.lock().unwrap().tokens()))
        }
        fn save(&self, t: &inoink::api::Tokens) -> Result<()> {
            let mut c = self.1.lock().unwrap();
            c.access_token = t.access_token.clone();
            c.refresh_token = t.refresh_token.clone();
            c.expires_at = t.expires_at;
            c.scope = t.scope.clone();
            inoink::cache::atomic_write(&self.0, &serde_json::to_vec_pretty(&*c)?)
        }
    }
    let store = PathTokens(PathBuf::from(&file), std::sync::Mutex::new(creds.clone()));
    let api = ApiClient::new(creds.app_config(), Arc::new(UreqTransport::new()), Arc::new(SystemClock), Arc::new(store))?
        .with_priority(inoink::api::Priority::Explicit);
    let mut failures = 0;
    let mut check = |name: &str, r: Result<String>| match r {
        Ok(d) => println!("PASS {name}: {d}"),
        Err(e) => {
            failures += 1;
            println!("FAIL {name}: {e}");
        }
    };
    check("user-info", api.user_info().map(|u| format!("user {}", u.user_id)));
    check("subscription/list", api.subscriptions().map(|s| format!("{} subscriptions", s.len())));
    check("tag/list", api.tags().map(|t| format!("{} tags/folders", t.len())));
    check("unread-count", api.unread_counts().map(|c| format!("{} streams", c.counts.len())));
    check("preference/stream/list", api.stream_preferences().map(|p| format!("{} ordering entries", p.len())));
    let mut ids = inoink::api::IdsParams::new(StreamId::new(STATE_STARRED));
    ids.count = 5;
    ids.only_starred = true;
    check("stream/items/ids", api.item_ids(&ids).map(|p| format!("{} ids", p.refs.len())));
    let params = inoink::api::ContentsParams { count: 2, annotations: true, summaries: true, exclude_read: true, ..Default::default() };
    check(
        "stream/contents",
        api.contents(&StreamId::new(STATE_READING_LIST), &params)
            .map(|p| format!("{} items, first has {} chars", p.items.len(), p.items.first().map(|i| i.summary_html.len()).unwrap_or(0))),
    );
    let q = api.quota();
    let z1 = q.remaining(inoink::api::Zone::One);
    println!("Zone 1 quota: {} of {} remaining ({}).", z1.remaining, z1.limit, if z1.estimated { "estimated" } else { "server-reported" });
    let _ = SystemClock.now();
    if failures > 0 {
        Err(Error::Data(format!("{failures} check(s) failed")))
    } else {
        Ok(())
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("authorize") => authorize(&args[1..]),
        Some("provision") => provision(&args[1..]),
        Some("fixtures") => fixtures(&args[1..]),
        Some("verify-cache") => verify_cache(&args[1..]),
        Some("acceptance") => acceptance(&args[1..]),
        _ => {
            eprint!("{USAGE}");
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
