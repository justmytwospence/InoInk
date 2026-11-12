//! The reading view of a linked page: extraction, refusal of http and non-HTML, short pages.

use inoink::cache::{Cache, CacheConfig, Control};
use inoink::content::extract::page_title;
use inoink::content::images::{Fetched, Fetcher};
use inoink::content::{prepare_article, ArticleMeta, AssetMap};
use inoink::error::Result;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use url::Url;

struct Pages(HashMap<String, Fetched>);

impl Fetcher for Pages {
    fn get(&self, url: &Url, _max: usize) -> Result<Fetched> {
        Ok(self.0.get(url.as_str()).cloned().unwrap_or(Fetched { status: 404, ..Default::default() }))
    }
}

fn html(body: &str) -> Fetched {
    Fetched { status: 200, content_type: Some("text/html; charset=utf-8".into()), location: None, body: body.as_bytes().to_vec() }
}

fn long_page() -> String {
    let para: String = (0..60).map(|i| format!("word{i} ")).collect();
    format!(
        "<html><head><title>  Plain   title </title><meta property=\"og:title\" content=\"The Real Title\"></head><body><nav>menu</nav>\
         <article><p>{para}</p><p>{para}</p><p>{para}</p></article></body></html>"
    )
}

fn cache() -> (tempfile::TempDir, Cache, Control) {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::new(&dir.path().join("cache"), "acct", CacheConfig::default());
    (dir, cache, Control::new(Arc::new(AtomicBool::new(false))))
}

#[test]
fn web_page_extracts_text_and_title() {
    let (_d, cache, ctl) = cache();
    let mut m = HashMap::new();
    m.insert("https://site.example/post".to_string(), html(&long_page()));
    let path = cache.web_page(&Pages(m), "https://site.example/post", &ctl).unwrap().unwrap();
    let out = std::fs::read_to_string(&path).unwrap();
    assert!(out.contains("<h1>The Real Title</h1>"), "{out}");
    assert!(out.contains("site.example"));
    assert!(out.contains("word10"));
    assert!(!out.contains("menu"), "navigation is left out");
    assert_eq!(path, cache.web_path("https://site.example/post"));
}

#[test]
fn web_page_refuses_http_and_non_html() {
    let (_d, cache, ctl) = cache();
    let mut m = HashMap::new();
    m.insert(
        "https://site.example/file".to_string(),
        Fetched { status: 200, content_type: Some("application/pdf".into()), location: None, body: b"%PDF".to_vec() },
    );
    let pages = Pages(m);
    assert_eq!(cache.web_page(&pages, "http://site.example/post", &ctl).unwrap().unwrap_err(), "only https pages can be opened here");
    assert_eq!(cache.web_page(&pages, "https://site.example/file", &ctl).unwrap().unwrap_err(), "the page is not HTML");
    assert!(cache.web_page(&pages, "not a url", &ctl).unwrap().is_err());
}

#[test]
fn web_page_reports_short_text() {
    let (_d, cache, ctl) = cache();
    let mut m = HashMap::new();
    m.insert("https://site.example/short".to_string(), html("<html><body><article><p>Just a few words here.</p></article></body></html>"));
    let why = cache.web_page(&Pages(m), "https://site.example/short", &ctl).unwrap().unwrap_err();
    assert!(why.contains("too little text") || why.contains("no article text"), "{why}");
}

#[test]
fn page_title_prefers_og_title() {
    assert_eq!(page_title(&long_page()).as_deref(), Some("The Real Title"));
    assert_eq!(page_title("<html><head><title>  Plain   title </title></head></html>").as_deref(), Some("Plain title"));
    assert_eq!(page_title("<html><body>nothing</body></html>"), None);
}

#[test]
fn prepare_article_output_has_the_documented_shape() {
    let meta = ArticleMeta {
        title: "T & U".into(),
        source: "S".into(),
        author: String::new(),
        date: "D".into(),
        url: String::new(),
        discussion: Some("12 comments".into()),
    };
    let out = prepare_article("<p>Hi</p>", None, &meta, &AssetMap::new()).html;
    assert_eq!(
        out,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>T &amp; U</title></head><body>\
         <h1>T &amp; U</h1><p class=\"inoink-meta\">S \u{b7} D</p><hr/><p>Hi</p>\
         <p class=\"inoink-switch\"><a href=\"inoink-comments:\">12 comments</a></p></body></html>\n"
    );
}
