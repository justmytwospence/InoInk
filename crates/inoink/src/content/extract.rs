//! Optional original-page extraction: pick the main content element of a fetched page.
//!
//! This is an InoInk feature, not an Inoreader API. It never sends credentials, only fetches
//! public https pages, and cannot recover paywalled or login-only text.

use super::images::{fetch_following, Fetcher, ImageLimits};
use crate::error::{Error, Result};
use html5ever::serialize::{serialize, SerializeOpts};
use html5ever::tendril::TendrilSink;
use html5ever::{parse_document, ParseOpts};
use markup5ever_rcdom::{Handle, NodeData, RcDom, SerializableHandle};
use std::sync::atomic::AtomicBool;
use url::Url;

/// Pages whose extracted text is shorter than this are reported as failed.
pub const MIN_TEXT_CHARS: usize = 400;

fn name(n: &Handle) -> Option<String> {
    if let NodeData::Element { name, .. } = &n.data {
        Some(name.local.to_string())
    } else {
        None
    }
}

fn attr(n: &Handle, key: &str) -> Option<String> {
    if let NodeData::Element { attrs, .. } = &n.data {
        attrs.borrow().iter().find(|a| &*a.name.local == key).map(|a| a.value.to_string())
    } else {
        None
    }
}

fn text_len(n: &Handle, depth: usize) -> usize {
    if depth > 64 {
        return 0;
    }
    match &n.data {
        NodeData::Text { contents } => contents.borrow().split_whitespace().map(|w| w.chars().count() + 1).sum(),
        NodeData::Element { .. } => {
            let nm = name(n).unwrap_or_default();
            if matches!(nm.as_str(), "script" | "style" | "nav" | "footer" | "aside" | "form" | "noscript") {
                return 0;
            }
            n.children.borrow().iter().map(|c| text_len(c, depth + 1)).sum()
        }
        _ => 0,
    }
}

/// Paragraph text only: the signal that distinguishes an article body from menus and widgets.
fn paragraph_score(n: &Handle, depth: usize) -> usize {
    if depth > 64 {
        return 0;
    }
    let mut score = 0;
    for c in n.children.borrow().iter() {
        match name(c).as_deref() {
            Some("p") | Some("pre") | Some("blockquote") | Some("li") => {
                let l = text_len(c, 0);
                if l >= 40 {
                    score += l;
                }
            }
            Some("script") | Some("style") | Some("nav") | Some("footer") | Some("aside") | Some("form") => {}
            Some(_) => score += paragraph_score(c, depth + 1),
            None => {}
        }
    }
    score
}

fn collect(n: &Handle, depth: usize, out: &mut Vec<(Handle, String, usize)>) {
    if depth > 64 {
        return;
    }
    for c in n.children.borrow().iter() {
        if let Some(nm) = name(c) {
            if matches!(nm.as_str(), "article" | "main" | "div" | "section") || attr(c, "role").as_deref() == Some("main") {
                let s = paragraph_score(c, 0);
                if s > 0 {
                    out.push((c.clone(), nm.clone(), s));
                }
            }
            collect(c, depth + 1, out);
        }
    }
}

/// The title of a page: its `og:title`, else its `<title>`, with whitespace collapsed.
pub fn page_title(html: &str) -> Option<String> {
    fn text_of(n: &Handle, out: &mut String) {
        if let NodeData::Text { contents } = &n.data {
            out.push_str(&contents.borrow());
        }
        for c in n.children.borrow().iter() {
            text_of(c, out);
        }
    }
    fn walk(n: &Handle, og: &mut Option<String>, title: &mut Option<String>) {
        if let Some(nm) = name(n) {
            if nm == "meta" && attr(n, "property").as_deref() == Some("og:title") && og.is_none() {
                *og = attr(n, "content");
            } else if nm == "title" && title.is_none() {
                let mut t = String::new();
                text_of(n, &mut t);
                *title = Some(t);
            }
        }
        for c in n.children.borrow().iter() {
            walk(c, og, title);
        }
    }
    let dom = parse_document(RcDom::default(), ParseOpts::default()).one(html);
    let (mut og, mut title) = (None, None);
    walk(&dom.document, &mut og, &mut title);
    let clean = |t: String| {
        let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
        (!t.is_empty()).then_some(t)
    };
    og.and_then(clean).or_else(|| title.and_then(clean))
}

/// Returns the inner HTML of the likely main content element, if there is one.
pub fn extract_main(html: &str) -> Option<String> {
    let dom = parse_document(RcDom::default(), ParseOpts::default()).one(html);
    let mut cands = Vec::new();
    collect(&dom.document, 0, &mut cands);
    let best = cands.iter().map(|(_, _, s)| *s).max()?;
    // Prefer a semantic element when it holds most of the text, else the strongest candidate
    // with the fewest descendants (the tightest container).
    let pick = cands
        .iter()
        .filter(|(_, n, s)| (n == "article" || n == "main") && *s * 10 >= best * 6)
        .min_by_key(|(_, _, s)| std::cmp::Reverse(*s))
        .or_else(|| cands.iter().filter(|(_, _, s)| *s * 10 >= best * 9).min_by_key(|(_, _, s)| *s))?;
    let mut out = Vec::new();
    serialize(&mut out, &SerializableHandle::from(pick.0.clone()), SerializeOpts::default()).ok()?;
    String::from_utf8(out).ok()
}

#[derive(Debug, PartialEq)]
pub enum Extraction {
    Extracted {
        html: String,
        chars: usize,
    },
    /// Nothing better than the feed content was found.
    Failed(String),
}

/// Fetches `page_url` (https only) and extracts its main content. `feed_chars` is the length of
/// the text (in characters) already available from the API; extraction must beat it.
pub fn extract_original(
    f: &dyn Fetcher,
    page_url: &str,
    feed_chars: usize,
    limits: &ImageLimits,
    cancel: &AtomicBool,
) -> Result<Extraction> {
    let url = Url::parse(page_url).map_err(|_| Error::InvalidRequest("bad article URL".into()))?;
    if url.scheme() != "https" {
        return Ok(Extraction::Failed("the original page is not served over https".into()));
    }
    let fetched = match fetch_following(f, &url, limits, cancel) {
        Ok(r) => r,
        Err(Error::InvalidRequest(m)) => return Ok(Extraction::Failed(format!("the page could not be loaded ({m})"))),
        Err(e) => return Err(e),
    };
    if !fetched.content_type.as_deref().map(|c| c.to_ascii_lowercase().contains("html")).unwrap_or(false) {
        return Ok(Extraction::Failed("the page is not HTML".into()));
    }
    let page = String::from_utf8_lossy(&fetched.body).into_owned();
    let Some(main) = extract_main(&page) else {
        return Ok(Extraction::Failed("no article text was found on the page".into()));
    };
    let probe = super::prepare_article(&main, Some(page_url), &super::ArticleMeta::default(), &super::AssetMap::new());
    let chars = probe.text.chars().count();
    if chars < MIN_TEXT_CHARS || chars <= feed_chars {
        return Ok(Extraction::Failed("the page has no more text than the feed already provided (it may need a login)".into()));
    }
    Ok(Extraction::Extracted { html: main, chars })
}
