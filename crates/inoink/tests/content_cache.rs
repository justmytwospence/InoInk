mod common;
use image::{ImageFormat, Luma, Rgb, RgbImage};
use inoink::cache::*;
use inoink::content::extract::*;
use inoink::content::images::*;
use inoink::content::*;
use inoink::error::{Error, Result};
use inoink::model::*;
use inoink::store::*;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::tempdir;
use url::Url;

fn png(w: u32, h: u32, color: [u8; 3]) -> Vec<u8> {
    let img = RgbImage::from_pixel(w, h, Rgb(color));
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).unwrap();
    out
}
fn jpeg(w: u32, h: u32) -> Vec<u8> {
    let img = RgbImage::from_pixel(w, h, Rgb([200, 30, 30]));
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), ImageFormat::Jpeg).unwrap();
    out
}
fn gif(w: u32, h: u32) -> Vec<u8> {
    let img = RgbImage::from_pixel(w, h, Rgb([0, 255, 0]));
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), ImageFormat::Gif).unwrap();
    out
}
fn webp(w: u32, h: u32) -> Vec<u8> {
    let img = RgbImage::from_pixel(w, h, Rgb([0, 0, 255]));
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), ImageFormat::WebP).unwrap();
    out
}

type Handler = Box<dyn Fn(&Url) -> Result<Fetched> + Send + Sync>;

struct MockFetcher {
    routes: Mutex<HashMap<String, Result<Fetched>>>,
    calls: Mutex<Vec<String>>,
    active: AtomicUsize,
    peak: AtomicUsize,
    delay: Duration,
    handler: Mutex<Option<Handler>>,
}

impl MockFetcher {
    fn new() -> MockFetcher {
        MockFetcher {
            routes: Mutex::new(HashMap::new()),
            calls: Mutex::new(vec![]),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            delay: Duration::ZERO,
            handler: Mutex::new(None),
        }
    }
    fn ok(&self, url: &str, body: Vec<u8>) {
        self.routes
            .lock()
            .unwrap()
            .insert(url.into(), Ok(Fetched { status: 200, content_type: Some("image/png".into()), location: None, body }));
    }
    fn ok_typed(&self, url: &str, content_type: &str, body: &str) {
        self.routes.lock().unwrap().insert(
            url.into(),
            Ok(Fetched { status: 200, content_type: Some(content_type.into()), location: None, body: body.as_bytes().to_vec() }),
        );
    }
    fn redirect(&self, url: &str, to: &str) {
        self.routes
            .lock()
            .unwrap()
            .insert(url.into(), Ok(Fetched { status: 302, content_type: None, location: Some(to.into()), body: vec![] }));
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl Fetcher for MockFetcher {
    fn get(&self, url: &Url, _max: usize) -> Result<Fetched> {
        self.calls.lock().unwrap().push(url.to_string());
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        if let Some(h) = self.handler.lock().unwrap().as_ref() {
            return h(url);
        }
        match self.routes.lock().unwrap().get(url.as_str()) {
            Some(Ok(f)) => Ok(f.clone()),
            Some(Err(Error::Offline)) => Err(Error::Offline),
            Some(Err(e)) => Err(Error::Retryable(e.to_string())),
            None => Ok(Fetched { status: 404, ..Default::default() }),
        }
    }
}

fn ctl() -> Control {
    let mut c = Control::new(Arc::new(AtomicBool::new(false)));
    c.sleep = Box::new(|_| {});
    c
}

fn rec(id: u64, ts: i64, html: &str) -> ArticleRecord {
    ArticleRecord {
        id,
        title: format!("Title {id}"),
        origin_stream: "feed/https://blog.example/rss".into(),
        origin_title: "Blog".into(),
        canonical_url: "https://blog.example/posts/1/".into(),
        published: 1_700_000_000,
        timestamp_usec: ts,
        summary_html: html.into(),
        raw_json: "{}".into(),
        ..Default::default()
    }
}

fn setup() -> (tempfile::TempDir, Store, Cache) {
    let dir = tempdir().unwrap();
    let store = Store::open(&dir.path().join("db.sqlite"), "acct").unwrap();
    let cache = Cache::new(&dir.path().join("cache"), "acct", CacheConfig::default());
    (dir, store, cache)
}

fn latest_html(cache: &Cache, store: &Store, id: u64) -> String {
    let v = store.latest_version(ArticleId(id)).unwrap().unwrap();
    std::fs::read_to_string(cache.version_path(ArticleId(id), v.version)).unwrap()
}

/// Balanced-tag check for the XHTML we emit.
fn assert_well_formed(html: &str) {
    let mut stack: Vec<String> = Vec::new();
    let mut i = 0;
    let b = html.as_bytes();
    while i < b.len() {
        if b[i] == b'<' {
            let end = html[i..].find('>').expect("unterminated tag") + i;
            let inner = &html[i + 1..end];
            if inner.starts_with('?') || inner.starts_with('!') {
            } else if let Some(name) = inner.strip_prefix('/') {
                assert_eq!(stack.pop().as_deref(), Some(name.trim()), "mismatched close in {html}");
            } else if !inner.ends_with('/') {
                stack.push(inner.split_whitespace().next().unwrap().to_string());
            }
            i = end + 1;
        } else {
            assert_ne!(b[i], b'>', "stray > in text");
            i += 1;
        }
    }
    assert!(stack.is_empty(), "unclosed: {stack:?}");
}

const BASE: &str = "https://blog.example/posts/1/";

#[test]
fn resolves_relative_lazy_and_responsive_images_in_order_with_captions() {
    let html = r#"
      <p>Intro</p>
      <img src="/img/a.png" alt="First &amp; best">
      <img src="data:image/gif;base64,R0lGOD" data-src="b.jpg" alt="Lazy">
      <img src="small.png" srcset="c-480.png 480w, c-1200.png 1200w, c-4000.png 4000w" alt="Responsive">
      <img data-srcset="//cdn.example/d-800.webp 800w, //cdn.example/d-400.webp 400w">
      <picture><source srcset="x.avif" type="image/avif"><img src="/p/e.png" alt=""></picture>
      <figure><img src="../f.png" alt="fig"><figcaption>A <b>bold</b> caption</figcaption></figure>
      <img src="/img/a.png" alt="dupe">
      <p>Outro</p>"#;
    let meta = ArticleMeta { title: "T".into(), ..Default::default() };
    let p = prepare_article(html, Some(BASE), &meta, &AssetMap::new());
    let urls: Vec<&str> = p.assets.iter().map(|a| a.url.as_str()).collect();
    assert_eq!(
        urls,
        vec![
            "https://blog.example/img/a.png",
            "https://blog.example/posts/1/b.jpg",
            "https://blog.example/posts/1/c-1200.png",
            "https://cdn.example/d-800.webp",
            "https://blog.example/p/e.png",
            "https://blog.example/posts/f.png",
        ],
        "document order, lazy attrs and srcset resolved, widest under 1600px chosen, duplicates folded"
    );
    assert_eq!(p.placeholders, 7);
    assert!(p.html.contains("A <b>bold</b> caption"));
    assert!(p.html.contains("inoink-caption"));
    assert!(p.text.contains("Intro") && p.text.contains("A bold caption") && p.text.contains("Outro"));
    assert!(p.html.find("Intro").unwrap() < p.html.find("Outro").unwrap());
    assert_well_formed(&p.html);
}

#[test]
fn sanitizer_removes_active_content_and_unsafe_urls() {
    let html = r##"
      <script>alert(1)</script><style>body{display:none}</style>
      <iframe src="https://evil.example"></iframe><form action="/x"><input name="a"><button>go</button></form>
      <object data="x"></object><embed src="x"><svg><script>1</script></svg><noscript><img src="https://t.example/pixel.gif"></noscript>
      <p onclick="steal()" style="color:red" class="x" id="keep-me" onmouseover="x">Hello <a href="javascript:alert(1)">bad</a>
         <a href="data:text/html,<script>1</script>">data</a> <a href="file:///etc/passwd">file</a>
         <a href="https://ok.example/p?a=1&b=2">good</a> <a href="#keep-me">frag</a> <a href="#bad id!">badfrag</a></p>
      <img src="file:///etc/passwd" alt="local"><img src="javascript:alert(1)"><img src="ftp://x/y.png" alt="ftp">
      <img src="https://ok.example/x.png" onerror="alert(1)" alt="a&quot;&gt;&lt;script&gt;">
      <a href="https://ok.example/&quot;onmouseover=&quot;x">q</a>
      <p>&lt;b&gt;not a tag&lt;/b&gt; &amp; text</p>"##;
    let p = prepare_article(html, Some(BASE), &ArticleMeta { title: "<T> & \"q\"".into(), ..Default::default() }, &AssetMap::new());
    let h = &p.html;
    for bad in [
        "<script",
        "alert(1)",
        "<style",
        "<iframe",
        "<form",
        "<input",
        "<button",
        "<object",
        "<embed",
        "<svg",
        "onclick",
        "onerror",
        " onmouseover",
        "\"onmouseover",
        "style=",
        "javascript:",
        "data:text",
        "file:",
        "ftp:",
        "evil.example",
        "t.example/pixel",
    ] {
        assert!(!h.contains(bad), "{bad} survived in {h}");
    }
    assert!(h.contains("id=\"keep-me\"") && h.contains("href=\"#keep-me\""));
    assert!(!h.contains("badfrag\"") || !h.contains("#bad id"));
    assert!(h.contains("href=\"https://ok.example/p?a=1&amp;b=2\""));
    assert!(h.contains("&lt;b&gt;not a tag&lt;/b&gt; &amp; text"), "text stays text");
    assert!(h.contains("<title>&lt;T&gt; &amp; &quot;q&quot;</title>"), "metadata is escaped");
    assert_eq!(p.assets.len(), 1, "only the https image is an asset");
    assert_well_formed(h);
}

#[test]
fn malformed_and_hostile_markup_still_yields_well_formed_output() {
    let nested = "<div>".repeat(5000) + "deep" + &"</div>".repeat(5000);
    for html in [
        "<p>unclosed <b>bold <i>both",
        "</div></p><td>stray cells</td>",
        "<table><tr><td>a<td>b<tr><td>c</table>",
        "<br><br/><hr>",
        "&nbsp;&#x41;&bogus; \u{0}\u{1}",
        "<a href='x'>",
        nested.as_str(),
        "",
    ] {
        let p = prepare_article(html, Some(BASE), &ArticleMeta::default(), &AssetMap::new());
        assert_well_formed(&p.html);
    }
}

#[test]
fn tables_and_code_blocks_keep_structure() {
    let html = "<table><caption>Cap</caption><thead><tr><th colspan=\"2\">H</th></tr></thead><tbody><tr><td>a</td><td rowspan=\"2\">b</td></tr></tbody></table><pre><code>fn main() {\n    println!(\"hi &amp; bye\");\n}\n</code></pre>";
    let p = prepare_article(html, None, &ArticleMeta::default(), &AssetMap::new());
    assert!(p.html.contains("<caption>Cap</caption>"));
    assert!(p.html.contains("<th colspan=\"2\">H</th>"));
    assert!(p.html.contains("<td rowspan=\"2\">b</td>"));
    assert!(p.html.contains("<pre><code>fn main() {\n    println!(\"hi &amp; bye\");\n}\n</code></pre>"));
    assert_well_formed(&p.html);
}

#[test]
fn caches_text_first_then_images_and_reaches_complete() {
    let (_d, mut store, cache) = setup();
    store
        .upsert_article(
            &rec(1, 100, r#"<p>Hello cafe</p><img src="/a.png" alt="A"><img src="b.gif" alt="B"><img src="/c.webp" alt="C">"#),
            0,
        )
        .unwrap();
    let f = MockFetcher::new();
    f.ok("https://blog.example/a.png", png(40, 20, [255, 0, 0]));
    f.ok("https://blog.example/posts/1/b.gif", gif(8, 8));
    f.ok("https://blog.example/c.webp", webp(10, 30));
    let state = cache.cache_article(&mut store, &f, ArticleId(1), &ctl()).unwrap();
    assert_eq!(state, DownloadState::Complete);
    assert_eq!(store.download_state(ArticleId(1)).unwrap(), DownloadState::Complete);
    let html = latest_html(&cache, &store, 1);
    assert!(!html.contains("inoink-missing"));
    assert_eq!(html.matches("<img src=\"assets/").count(), 3);
    // Normalization: GIF and WebP become PNG, aspect ratio unchanged, colour preserved.
    let dir = cache.article_dir(ArticleId(1)).join("assets");
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    files.sort();
    assert_eq!(files.len(), 3);
    assert!(files.iter().all(|f| f.ends_with(".png")), "{files:?}");
    let a = image::open(dir.join(format!("{}.png", asset_name("https://blog.example/a.png")))).unwrap().to_rgb8();
    assert_eq!((a.width(), a.height()), (40, 20));
    assert_eq!(a.get_pixel(0, 0).0, [255, 0, 0], "colour is preserved");
    let c = image::open(dir.join(format!("{}.png", asset_name("https://blog.example/c.webp")))).unwrap();
    assert_eq!((c.width(), c.height()), (10, 30));
    // Text is searchable offline.
    let hits = store.query_articles(&ArticleQuery { search: Some("CAFE".into()), ..Default::default() }).unwrap();
    assert_eq!(hits.items.len(), 1);
    // Only the final version remains once nothing is leased.
    assert_eq!(store.versions(ArticleId(1)).unwrap().len(), 1);
    assert!(cache.verify(&store).unwrap().problems.is_empty());
}

#[test]
fn oversize_jpeg_is_downscaled_and_png_passes_through() {
    let n = normalize_image(&jpeg(2400, 1200), &ImageLimits::default()).unwrap();
    assert_eq!(n.ext, "jpg");
    assert_eq!((n.width, n.height), (1800, 900), "aspect ratio kept when scaling down");
    let original = png(30, 30, [1, 2, 3]);
    let n = normalize_image(&original, &ImageLimits::default()).unwrap();
    assert_eq!(n.bytes, original);
}

#[test]
fn unsupported_formats_decode_bombs_and_bad_mime_become_failed_placeholders() {
    let (_d, mut store, cache) = setup();
    store.upsert_article(&rec(1, 100, r#"<img src="/s.svg" alt="svg"><img src="/bomb.png" alt="bomb"><img src="/page.png" alt="html"><img src="/ok.png" alt="ok">"#), 0).unwrap();
    let f = MockFetcher::new();
    f.ok("https://blog.example/s.svg", b"<svg xmlns='http://www.w3.org/2000/svg'><script>1</script></svg>".to_vec());
    let bomb = {
        let img = image::ImageBuffer::<Luma<u8>, Vec<u8>>::new(6000, 6001);
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).unwrap();
        out
    };
    assert!(bomb.len() < 200_000, "the bomb is small on the wire");
    f.ok("https://blog.example/bomb.png", bomb);
    f.ok("https://blog.example/page.png", b"<html><body>Please log in</body></html>".to_vec());
    f.ok("https://blog.example/ok.png", png(5, 5, [9, 9, 9]));
    let state = cache.cache_article(&mut store, &f, ArticleId(1), &ctl()).unwrap();
    assert_eq!(state, DownloadState::MissingImages);
    let html = latest_html(&cache, &store, 1);
    assert_eq!(html.matches("Image unavailable").count(), 3);
    assert_eq!(html.matches("<img src=\"assets/").count(), 1);
    assert!(html.contains("inoink-retry:"), "failed images are retryable placeholders, not silently dropped");
    assert!(html.contains(">svg<") || html.contains("svg</p>"), "alt text is kept with the placeholder");
    assert!(DownloadState::MissingImages.readable_offline());
    // Permanent failures are not re-fetched automatically...
    let calls_before = f.calls().len();
    cache.cache_article(&mut store, &f, ArticleId(1), &ctl()).unwrap();
    assert_eq!(f.calls().len(), calls_before);
    // ...but the user can retry one.
    f.ok("https://blog.example/s.svg", png(6, 6, [0, 0, 0]));
    let name = asset_name("https://blog.example/s.svg");
    let state = cache.retry_asset(&mut store, &f, ArticleId(1), &name, &ctl()).unwrap();
    assert_eq!(state, DownloadState::MissingImages);
    assert_eq!(latest_html(&cache, &store, 1).matches("Image unavailable").count(), 2);
}

#[test]
fn private_addresses_and_redirects_to_them_are_never_fetched() {
    for u in [
        "http://127.0.0.1/a.png",
        "http://localhost/a.png",
        "http://[::1]/a.png",
        "http://169.254.169.254/latest",
        "http://10.0.0.5/x",
        "http://192.168.1.1/x",
        "http://2130706433/x",
        "http://[::ffff:127.0.0.1]/x",
        "http://printer.local/x",
        "http://intranet/x",
        "file:///etc/passwd",
        "ftp://example.org/x",
    ] {
        assert!(is_blocked_url(&Url::parse(u).unwrap()), "{u}");
    }
    for u in ["https://example.org/a.png", "http://8.8.8.8/x", "https://cdn.example.co.uk/x"] {
        assert!(!is_blocked_url(&Url::parse(u).unwrap()), "{u}");
    }
    let f = MockFetcher::new();
    f.redirect("https://blog.example/redir.png", "http://169.254.169.254/latest/meta-data");
    f.ok("http://169.254.169.254/latest/meta-data", png(2, 2, [0, 0, 0]));
    let cancel = AtomicBool::new(false);
    let r = download_image(&f, &Url::parse("https://blog.example/redir.png").unwrap(), &ImageLimits::default(), &cancel, &|_| {});
    assert!(matches!(r, Err(DownloadError::Permanent(_))));
    assert_eq!(f.calls(), vec!["https://blog.example/redir.png"], "the blocked hop is never requested");

    // Redirect loops and long chains are bounded.
    f.redirect("https://a.example/1", "https://a.example/2");
    f.redirect("https://a.example/2", "https://a.example/1");
    let r = download_image(&f, &Url::parse("https://a.example/1").unwrap(), &ImageLimits::default(), &cancel, &|_| {});
    assert!(matches!(r, Err(DownloadError::Permanent(_))));
    assert!(f.calls().len() <= 1 + ImageLimits::default().max_redirects + 1);
}

#[test]
fn transient_failures_are_retried_with_backoff_then_left_pending() {
    let f = MockFetcher::new();
    let tries = Arc::new(AtomicUsize::new(0));
    let t = tries.clone();
    let png_bytes = png(3, 3, [1, 1, 1]);
    *f.handler.lock().unwrap() = Some(Box::new(move |_u| {
        if t.fetch_add(1, Ordering::SeqCst) < 2 {
            Err(Error::Retryable("503".into()))
        } else {
            Ok(Fetched { status: 200, content_type: Some("image/png".into()), location: None, body: png_bytes.clone() })
        }
    }));
    let sleeps = Mutex::new(Vec::new());
    let cancel = AtomicBool::new(false);
    let url = Url::parse("https://blog.example/x.png").unwrap();
    let n = download_image(&f, &url, &ImageLimits::default(), &cancel, &|d| sleeps.lock().unwrap().push(d)).unwrap();
    assert_eq!(n.ext, "png");
    assert_eq!(tries.load(Ordering::SeqCst), 3);
    assert_eq!(*sleeps.lock().unwrap(), vec![Duration::from_millis(500), Duration::from_millis(1000)]);

    // Persistent failure: three attempts, then transient (not permanent).
    let g = MockFetcher::new();
    *g.handler.lock().unwrap() = Some(Box::new(|_| Err(Error::Offline)));
    let r = download_image(&g, &url, &ImageLimits::default(), &cancel, &|_| {});
    assert_eq!(r, Err(DownloadError::Transient("offline".into())));
    assert_eq!(g.calls().len(), 3);

    // In the cache, such an image stays pending and the article is text-ready.
    let (_d, mut store, cache) = setup();
    store.upsert_article(&rec(1, 1, r#"<p>text</p><img src="/x.png" alt="x">"#), 0).unwrap();
    let state = cache.cache_article(&mut store, &g, ArticleId(1), &ctl()).unwrap();
    assert_eq!(state, DownloadState::TextReady);
    assert!(latest_html(&cache, &store, 1).contains("Image not downloaded"));
    assert_eq!(store.assets(ArticleId(1)).unwrap()[0].state, "pending");
}

#[test]
fn downloads_are_concurrency_bounded_and_cancellable() {
    let (_d, mut store, cache) = setup();
    let imgs: String = (0..8).map(|i| format!("<img src=\"/i{i}.png\" alt=\"{i}\">")).collect();
    store.upsert_article(&rec(1, 1, &imgs), 0).unwrap();
    let mut f = MockFetcher::new();
    f.delay = Duration::from_millis(40);
    for i in 0..8 {
        f.ok(&format!("https://blog.example/i{i}.png"), png(4, 4, [i as u8, 0, 0]));
    }
    let state = cache.cache_article(&mut store, &f, ArticleId(1), &ctl()).unwrap();
    assert_eq!(state, DownloadState::Complete);
    let peak = f.peak.load(Ordering::SeqCst);
    assert!((1..=2).contains(&peak), "peak concurrency {peak}");

    // Cancellation before downloads: text is published, images stay pending, nothing is lost.
    store.upsert_article(&rec(2, 2, &imgs), 0).unwrap();
    let c = Control { cancel: Arc::new(AtomicBool::new(true)), sleep: Box::new(|_| {}), free_space: Box::new(|| None) };
    let f2 = MockFetcher::new();
    let state = cache.cache_article(&mut store, &f2, ArticleId(2), &c).unwrap();
    assert_eq!(state, DownloadState::TextReady);
    assert!(f2.calls().is_empty());
    assert_eq!(store.assets(ArticleId(2)).unwrap().iter().filter(|a| a.state == "pending").count(), 8);
}

#[test]
fn leases_keep_the_version_a_reader_has_open() {
    let (_d, mut store, cache) = setup();
    store.upsert_article(&rec(1, 1, r#"<p>t</p><img src="/a.png" alt="a">"#), 0).unwrap();
    let f = MockFetcher::new();
    // First pass fails transiently: text-ready v1.
    *f.handler.lock().unwrap() = Some(Box::new(|_| Err(Error::Offline)));
    cache.cache_article(&mut store, &f, ArticleId(1), &ctl()).unwrap();
    let v1 = store.latest_version(ArticleId(1)).unwrap().unwrap();
    store.lease(ArticleId(1), v1.version).unwrap();
    // The reader holds v1 open while images arrive.
    *f.handler.lock().unwrap() = None;
    f.ok("https://blog.example/a.png", png(5, 5, [1, 2, 3]));
    let state = cache.cache_article(&mut store, &f, ArticleId(1), &ctl()).unwrap();
    assert_eq!(state, DownloadState::Complete);
    let versions = store.versions(ArticleId(1)).unwrap();
    assert_eq!(versions.len(), 2, "leased version kept alongside the new one");
    assert!(cache.version_path(ArticleId(1), v1.version).is_file());
    assert_ne!(versions[0].hash, versions[1].hash);
    // Releasing the lease lets garbage collection drop it.
    store.release_lease(ArticleId(1), v1.version).unwrap();
    cache.collect_garbage(&mut store, ArticleId(1)).unwrap();
    assert_eq!(store.versions(ArticleId(1)).unwrap().len(), 1);
    assert!(!cache.version_path(ArticleId(1), v1.version).exists());
}

#[test]
fn position_anchors_relocate_across_content_versions() {
    let old = "<p>alpha</p><p>The quick brown fox</p><p>omega</p>";
    let new = "<p>NEW HEADER IMAGE PLACEHOLDER LONG TEXT</p><p>alpha</p><p>The quick brown fox</p><p>omega</p><p>The quick brown fox</p>";
    let old_off = old.find("The quick").unwrap();
    let at = relocate(new, "The quick brown fox", old_off).unwrap();
    assert_eq!(&new[at..at + 9], "The quick");
    assert!(at < new.rfind("The quick").unwrap(), "nearest occurrence to the old offset wins");
    assert_eq!(relocate(new, "gone entirely", old_off), None);
    assert_eq!(relocate(new, "   ", old_off), None);
}

#[test]
fn recovery_removes_partial_writes_and_orphans_and_never_claims_missing_content() {
    let (_d, mut store, cache) = setup();
    store.upsert_article(&rec(1, 1, r#"<p>t</p><img src="/a.png" alt="a">"#), 0).unwrap();
    store.upsert_article(&rec(2, 2, "<p>two</p>"), 0).unwrap();
    let f = MockFetcher::new();
    f.ok("https://blog.example/a.png", png(5, 5, [1, 2, 3]));
    cache.cache_article(&mut store, &f, ArticleId(1), &ctl()).unwrap();
    cache.cache_article(&mut store, &f, ArticleId(2), &ctl()).unwrap();

    // Simulate an interrupted run: partial files, an unpublished version, an unknown asset.
    let d1 = cache.article_dir(ArticleId(1));
    std::fs::write(d1.join("assets").join("deadbeef.png.part"), b"half").unwrap();
    std::fs::write(d1.join("assets").join("unknown.png"), b"x").unwrap();
    std::fs::write(d1.join("v9.html"), b"<p>never published</p>").unwrap();
    std::fs::write(d1.join("v10.html.part"), b"x").unwrap();
    // And article 2's file vanished (e.g. storage corruption).
    let v2 = store.latest_version(ArticleId(2)).unwrap().unwrap();
    std::fs::remove_file(cache.version_path(ArticleId(2), v2.version)).unwrap();
    assert!(!cache.verify(&store).unwrap().problems.is_empty());

    let rep = cache.recover(&mut store).unwrap();
    assert_eq!(rep.removed_partial, 2);
    assert_eq!(rep.removed_orphans, 2);
    assert_eq!(rep.dropped_missing_versions, 1);
    assert!(!d1.join("v9.html").exists());
    assert!(d1.join(format!("v{}.html", store.latest_version(ArticleId(1)).unwrap().unwrap().version)).is_file());
    assert_eq!(store.download_state(ArticleId(2)).unwrap(), DownloadState::NotDownloaded, "never reported offline-ready without files");
    assert!(cache.verify(&store).unwrap().problems.is_empty());
}

#[test]
fn symlinks_in_the_cache_are_removed_not_followed() {
    let (d, mut store, cache) = setup();
    store.upsert_article(&rec(1, 1, "<p>t</p>"), 0).unwrap();
    let f = MockFetcher::new();
    cache.cache_article(&mut store, &f, ArticleId(1), &ctl()).unwrap();
    let secret = d.path().join("secret.txt");
    std::fs::write(&secret, b"top secret").unwrap();
    let assets = cache.article_dir(ArticleId(1)).join("assets");
    std::fs::create_dir_all(&assets).unwrap();
    std::os::unix::fs::symlink(&secret, assets.join("evil.png")).unwrap();
    cache.recover(&mut store).unwrap();
    assert!(!assets.join("evil.png").exists());
    assert_eq!(std::fs::read(&secret).unwrap(), b"top secret", "the link target is untouched");
}

#[test]
fn every_written_path_stays_inside_the_cache_root() {
    let (d, mut store, cache) = setup();
    let html = r#"<img src="../../../../etc/passwd" alt="x"><img src="https://blog.example/%2e%2e/%2e%2e/evil.png"><img src="https://blog.example/a/../../b.png">"#;
    store.upsert_article(&rec(1, 1, html), 0).unwrap();
    let f = MockFetcher::new();
    *f.handler.lock().unwrap() =
        Some(Box::new(|_| Ok(Fetched { status: 200, content_type: None, location: None, body: png(2, 2, [1, 1, 1]) })));
    cache.cache_article(&mut store, &f, ArticleId(1), &ctl()).unwrap();
    let root = cache.root().canonicalize().unwrap();
    let mut n = 0;
    let mut stack = vec![d.path().join("cache")];
    while let Some(p) = stack.pop() {
        for e in std::fs::read_dir(&p).unwrap().flatten() {
            let path = e.path();
            assert!(path.canonicalize().unwrap().starts_with(&root), "{path:?} escaped");
            n += 1;
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    assert!(n > 3);
    // Asset names are pure hashes: no server-provided names reach the filesystem.
    for a in store.assets(ArticleId(1)).unwrap() {
        assert!(a.name.len() == 32 && a.name.chars().all(|c| c.is_ascii_hexdigit()));
    }
}

#[test]
fn storage_pressure_budget_protection_and_pause() {
    let (_d, mut store, _c) = setup();
    let dir = _d.path().join("cache2");
    let cfg = CacheConfig { budget_bytes: 10_000, ..Default::default() };
    let cache = Cache::new(&dir, "acct", cfg);
    let f = MockFetcher::new();
    // Five cached articles of increasing age, each with an image of a few KB.
    for n in 1..=5u64 {
        store.upsert_article(&rec(n, n as i64 * 1000, &format!("<p>text {n}</p><img src=\"/i{n}.png\" alt=\"\">")), 0).unwrap();
        f.ok(&format!("https://blog.example/i{n}.png"), png(60, 60, [n as u8 * 10, 5, 5]));
        cache.cache_article(&mut store, &f, ArticleId(n), &ctl()).unwrap();
    }
    let total: i64 = store.cache_usage().unwrap().iter().map(|e| e.bytes).sum();
    assert!(total > 0);
    // Protect: #1 starred (oldest!), #2 has a pending action, #3 is leased.
    store.apply_intent(ArticleId(1), &Membership::Starred, true).unwrap();
    store.apply_intent(ArticleId(2), &Membership::Read, true).unwrap();
    store.lease(ArticleId(3), 1).unwrap();
    let cache = Cache::new(&dir, "acct", CacheConfig { budget_bytes: 1, ..Default::default() });
    let rep = cache.evict_to_budget(&mut store).unwrap();
    assert_eq!(rep.evicted, vec![ArticleId(4), ArticleId(5)], "oldest unprotected go first; protected stay");
    assert!(rep.protected_over_budget);
    for n in [1, 2, 3] {
        assert!(store.download_state(ArticleId(n)).unwrap().readable_offline(), "#{n} protected");
    }
    for n in [4, 5] {
        assert_eq!(
            store.download_state(ArticleId(n)).unwrap(),
            DownloadState::NotDownloaded,
            "evicted content is never presented as offline-ready"
        );
        assert!(!cache.article_dir(ArticleId(n)).exists());
        assert!(store.has_article(ArticleId(n)).unwrap(), "metadata is kept");
    }
    // Pending actions survive eviction.
    assert_eq!(store.pending_for(ArticleId(2)).unwrap().len(), 1);
    // Automatic downloads pause when protected content alone fills the budget.
    let plan = cache.plan_downloads(&mut store, 10_000_000).unwrap();
    assert_eq!(plan, DownloadPlan { ids: vec![], paused: Some(PauseReason::StorageLimit) });
}

#[test]
fn download_plan_prioritizes_starred_pinned_then_recent_unread() {
    let (_d, mut store, _c) = setup();
    let now = 1_800_000_000i64;
    let usec = |days_ago: i64| (now - days_ago * 86400) * 1_000_000;
    store.upsert_article(&rec(1, usec(1), "<p>recent unread</p>"), 0).unwrap();
    store.upsert_article(&rec(2, usec(90), "<p>old starred</p>"), 0).unwrap();
    store.upsert_article(&rec(3, usec(90), "<p>old unread</p>"), 0).unwrap();
    store.upsert_article(&rec(4, usec(2), "<p>recent read</p>"), 0).unwrap();
    let mut pinned = rec(5, usec(60), "<p>old pinned feed</p>");
    pinned.origin_stream = "feed/https://pinned.example/rss".into();
    store.upsert_article(&pinned, 0).unwrap();
    store.apply_intent(ArticleId(2), &Membership::Starred, true).unwrap();
    store.apply_intent(ArticleId(4), &Membership::Read, true).unwrap();
    store
        .replace_subscriptions(&[Subscription {
            id: "feed/https://pinned.example/rss".into(),
            title: "P".into(),
            url: String::new(),
            html_url: String::new(),
            sort_id: String::new(),
            first_item_usec: 0,
            folders: vec!["Keep".into()],
        }])
        .unwrap();
    let cache = Cache::new(&_d.path().join("c"), "acct", CacheConfig { pinned_folders: vec!["Keep".into()], ..Default::default() });
    let plan = cache.plan_downloads(&mut store, now).unwrap();
    assert_eq!(plan.ids, vec![ArticleId(2), ArticleId(5), ArticleId(1)]);
    assert!(plan.paused.is_none());
}

#[test]
fn low_free_space_refuses_new_downloads_but_keeps_existing_content() {
    let (_d, mut store, cache) = setup();
    store.upsert_article(&rec(1, 1, "<p>one</p>"), 0).unwrap();
    store.upsert_article(&rec(2, 2, "<p>two</p>"), 0).unwrap();
    let f = MockFetcher::new();
    cache.cache_article(&mut store, &f, ArticleId(1), &ctl()).unwrap();
    let mut c = ctl();
    c.free_space = Box::new(|| Some(1024));
    assert!(matches!(cache.cache_article(&mut store, &f, ArticleId(2), &c), Err(Error::Storage(_))));
    assert_eq!(store.download_state(ArticleId(2)).unwrap(), DownloadState::NotDownloaded);
    assert!(store.download_state(ArticleId(1)).unwrap().readable_offline());
    assert!(free_space_of(_d.path()).is_some());
}

#[test]
fn atomic_write_leaves_no_partial_file_on_failure() {
    let d = tempdir().unwrap();
    let target = d.path().join("sub").join("file.bin");
    atomic_write(&target, b"hello").unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"hello");
    // A directory in the way makes the final rename fail; no .part file is left.
    let blocked = d.path().join("blocked");
    std::fs::create_dir_all(blocked.join("child")).unwrap();
    assert!(atomic_write(&blocked, b"x").is_err());
    assert!(!d.path().join("blocked.part").exists());
    assert_eq!(format_date(1_700_000_000), "2023-11-14");
    assert_eq!(format_date(0), "");
}

struct PageFetcher(Fetched);
impl Fetcher for PageFetcher {
    fn get(&self, _u: &Url, _m: usize) -> Result<Fetched> {
        Ok(self.0.clone())
    }
}

fn page(body: &str, ct: &str) -> PageFetcher {
    PageFetcher(Fetched { status: 200, content_type: Some(ct.into()), location: None, body: body.as_bytes().to_vec() })
}

#[test]
fn original_page_extraction_succeeds_only_when_it_adds_text() {
    let para = "This is a long paragraph with enough words to look like genuine article content for the extractor. ".repeat(3);
    let article_page = format!("<html><body><nav>{}</nav><div class='sidebar'><p>short</p></div><article><h1>T</h1><p>{para}</p><p>{para}</p></article><footer>{}</footer></body></html>", "menu ".repeat(50), "legal ".repeat(40));
    let cancel = AtomicBool::new(false);
    let lim = ImageLimits::default();
    match extract_original(&page(&article_page, "text/html; charset=utf-8"), "https://blog.example/p", 100, &lim, &cancel).unwrap() {
        Extraction::Extracted { html, chars } => {
            assert!(html.contains("genuine article content"));
            assert!(!html.contains("menu menu") && !html.contains("legal legal"));
            assert!(chars > 400);
        }
        other => panic!("{other:?}"),
    }
    // Feed content was already as long: not an improvement.
    assert!(matches!(
        extract_original(&page(&article_page, "text/html"), "https://blog.example/p", 100_000, &lim, &cancel).unwrap(),
        Extraction::Failed(_)
    ));
    // A login wall has too little text.
    let wall = "<html><body><div><p>Please sign in to continue reading this article. Subscribers only.</p></div></body></html>";
    assert!(
        matches!(extract_original(&page(wall, "text/html"), "https://blog.example/p", 10, &lim, &cancel).unwrap(), Extraction::Failed(m) if m.contains("login") || m.contains("more text"))
    );
    // Non-https, non-HTML and unfetchable targets fail without side effects.
    assert!(matches!(
        extract_original(&page(&article_page, "text/html"), "http://blog.example/p", 10, &lim, &cancel).unwrap(),
        Extraction::Failed(_)
    ));
    assert!(matches!(
        extract_original(&page(&article_page, "application/pdf"), "https://blog.example/p", 10, &lim, &cancel).unwrap(),
        Extraction::Failed(_)
    ));
    assert!(matches!(
        extract_original(&page(&article_page, "text/html"), "https://127.0.0.1/p", 10, &lim, &cancel).unwrap(),
        Extraction::Failed(_)
    ));
}

#[test]
fn extracted_content_is_a_new_version_and_the_feed_source_is_preserved() {
    let (_d, mut store, cache) = setup();
    store.upsert_article(&rec(1, 1, "<p>short feed text</p>"), 0).unwrap();
    let f = MockFetcher::new();
    cache.cache_article(&mut store, &f, ArticleId(1), &ctl()).unwrap();
    let v1 = store.latest_version(ArticleId(1)).unwrap().unwrap();
    cache
        .publish_extracted(
            &mut store,
            &f,
            ArticleId(1),
            "<p>the whole long original page text</p>",
            "https://blog.example/posts/1/",
            &ctl(),
        )
        .unwrap();
    let v2 = store.latest_version(ArticleId(1)).unwrap().unwrap();
    assert!(v2.version > v1.version);
    assert!(v2.raw_hash.starts_with("extract:"));
    assert_eq!(store.article_record(ArticleId(1)).unwrap().unwrap().summary_html, "<p>short feed text</p>", "API content is untouched");
    assert!(latest_html(&cache, &store, 1).contains("whole long original page"));
}

#[test]
fn word_counts_and_read_time() {
    use inoink::content::{format_read_time, word_count, word_count_html};
    assert_eq!(word_count("Hello, world  -- 42"), 3);
    assert_eq!(word_count_html("<p>One <b>two</b></p><script>x y z</script>"), 2);
    assert_eq!(word_count_html("<style>p{a:b}</style><p>Un café</p>"), 2);
    assert_eq!(format_read_time(0), None);
    assert_eq!(format_read_time(1).unwrap(), "1 min read");
    assert_eq!(format_read_time(230).unwrap(), "1 min read");
    assert_eq!(format_read_time(231).unwrap(), "2 min read");
    assert_eq!(format_read_time(13800).unwrap(), "1 h read");
    assert_eq!(format_read_time(14030).unwrap(), "1 h 1 min read");
}

// ---- discussion link posts -------------------------------------------------------------------

const HN_SUMMARY: &str = "<a href=\"https://news.ycombinator.com/item?id=4242\">Comments</a>";
const HN_API: &str = "https://hn.algolia.com/api/v1/items/4242";
const STORY_URL: &str = "https://blog.example/posts/story";

fn hn_json(extra: usize) -> String {
    let mut kids = vec![
        r#"{"type":"comment","author":"ann","text":"<p>First</p>","created_at_i":1700000000,"children":[{"type":"comment","author":"bob","text":"<p>Reply</p>","created_at_i":1700000100,"children":[]}]}"#.to_string(),
    ];
    for i in 0..extra {
        kids.push(format!(r#"{{"type":"comment","author":"u{i}","text":"<p>More {i}</p>","created_at_i":1700000200,"children":[]}}"#));
    }
    format!(r#"{{"type":"story","title":"A story","url":"{STORY_URL}","text":null,"children":[{}]}}"#, kids.join(","))
}

fn story_page() -> String {
    let para = "This is a long paragraph with enough words to look like genuine article content for the extractor. ".repeat(6);
    format!(
        "<html><body><nav>{}</nav><article><h1>Story</h1><p>{para}</p><p>{para}</p><p>{para}</p></article></body></html>",
        "menu ".repeat(20)
    )
}

fn link_post_record(id: u64) -> ArticleRecord {
    let mut r = rec(id, 10, HN_SUMMARY);
    r.canonical_url = STORY_URL.into();
    r.origin_title = "Hacker News".into();
    r
}

#[test]
fn hn_link_post_caches_full_article_and_comments() {
    let (_d, mut store, cache) = setup();
    store.upsert_article(&link_post_record(1), 0).unwrap();
    let f = MockFetcher::new();
    f.ok_typed(HN_API, "application/json", &hn_json(0));
    f.ok_typed(STORY_URL, "text/html; charset=utf-8", &story_page());
    cache.cache_article(&mut store, &f, ArticleId(1), &ctl()).unwrap();
    let html = latest_html(&cache, &store, 1);
    assert!(html.contains("genuine article content"), "{html}");
    assert!(html.contains("inoink-comments:"));
    assert!(cache.comments_path(ArticleId(1)).is_file());
    let row = store.discussion(ArticleId(1)).unwrap().unwrap();
    assert_eq!(row.comment_count, 2);
    assert!(row.fetched > 0);
    let words = store.query_articles(&ArticleQuery::default()).unwrap().items[0].words;
    assert!(words > 200, "{words}");
    // A second cache pass uses the stored article and comments: no new network calls.
    let calls = f.calls().len();
    cache.cache_article(&mut store, &f, ArticleId(1), &ctl()).unwrap();
    assert_eq!(f.calls().len(), calls);
}

#[test]
fn failed_extraction_falls_back_then_retries() {
    let (_d, mut store, cache) = setup();
    store.upsert_article(&link_post_record(1), 0).unwrap();
    let f = MockFetcher::new();
    f.ok_typed(HN_API, "application/json", &hn_json(0));
    cache.cache_article(&mut store, &f, ArticleId(1), &ctl()).unwrap();
    let html = latest_html(&cache, &store, 1);
    assert!(html.contains("could not be downloaded") && html.contains(STORY_URL), "{html}");
    assert!(cache.comments_path(ArticleId(1)).is_file());
    // The failure was not stored: evict and try again with the page available.
    store.clear_content(ArticleId(1)).unwrap();
    f.ok_typed(STORY_URL, "text/html; charset=utf-8", &story_page());
    cache.cache_article(&mut store, &f, ArticleId(1), &ctl()).unwrap();
    assert!(latest_html(&cache, &store, 1).contains("genuine article content"));
}

#[test]
fn thread_failure_still_caches_the_article() {
    let (_d, mut store, cache) = setup();
    store.upsert_article(&link_post_record(1), 0).unwrap();
    let f = MockFetcher::new();
    f.ok_typed(STORY_URL, "text/html; charset=utf-8", &story_page());
    cache.cache_article(&mut store, &f, ArticleId(1), &ctl()).unwrap();
    assert!(latest_html(&cache, &store, 1).contains("genuine article content"));
    assert!(!cache.comments_path(ArticleId(1)).exists());
    assert_eq!(store.discussion(ArticleId(1)).unwrap().unwrap().fetched, 0);
}

#[test]
fn refresh_comments_updates_the_snapshot() {
    let (_d, mut store, cache) = setup();
    store.upsert_article(&link_post_record(1), 0).unwrap();
    let f = MockFetcher::new();
    f.ok_typed(HN_API, "application/json", &hn_json(0));
    f.ok_typed(STORY_URL, "text/html; charset=utf-8", &story_page());
    cache.cache_article(&mut store, &f, ArticleId(1), &ctl()).unwrap();
    f.ok_typed(HN_API, "application/json", &hn_json(1));
    let n = cache.refresh_comments(&mut store, &f, ArticleId(1), &ctl()).unwrap();
    assert_eq!(n, 3);
    assert_eq!(store.discussion(ArticleId(1)).unwrap().unwrap().comment_count, 3);
}

#[test]
fn eviction_counts_and_clears_comments() {
    let (_d, mut store, cache) = setup();
    store.upsert_article(&link_post_record(1), 0).unwrap();
    let f = MockFetcher::new();
    f.ok_typed(HN_API, "application/json", &hn_json(0));
    f.ok_typed(STORY_URL, "text/html; charset=utf-8", &story_page());
    cache.cache_article(&mut store, &f, ArticleId(1), &ctl()).unwrap();
    let row = store.discussion(ArticleId(1)).unwrap().unwrap();
    assert!(row.comments_bytes > 0);
    let usage = store.cache_usage().unwrap();
    assert!(usage[0].bytes >= row.comments_bytes);
    store.clear_content(ArticleId(1)).unwrap();
    let row = store.discussion(ArticleId(1)).unwrap().unwrap();
    assert_eq!((row.fetched, row.comments_bytes), (0, 0));
}
