//! Content pipeline: sanitize API HTML into XHTML that Plato's engine renders, resolve and
//! download images, and keep every file under a rooted cache directory.

pub mod discussion;
pub mod extract;
pub mod images;
pub mod sanitize;

use std::collections::HashMap;

/// Words in plain text: whitespace-separated tokens with at least one letter or digit.
pub fn word_count(text: &str) -> u32 {
    text.split_whitespace().filter(|t| t.chars().any(|c| c.is_alphanumeric())).count() as u32
}

/// Words in an HTML fragment: tags are removed and script/style contents ignored.
pub fn word_count_html(html: &str) -> u32 {
    let lower = html.to_ascii_lowercase();
    let mut text = String::with_capacity(html.len());
    let mut i = 0;
    let bytes = html.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let skip_to = ["script", "style"].iter().find_map(|tag| {
                if lower[i + 1..].starts_with(tag) {
                    let close = format!("</{tag}");
                    Some(lower[i..].find(&close).map(|p| i + p).unwrap_or(bytes.len()))
                } else {
                    None
                }
            });
            if let Some(end) = skip_to {
                // Resume at the closing tag; its own '>' ends the tag below.
                i = end;
                if i >= bytes.len() {
                    break;
                }
            }
            match html[i..].find('>') {
                Some(p) => i += p + 1,
                None => break,
            }
            text.push(' ');
        } else {
            let ch = html[i..].chars().next().unwrap();
            text.push(ch);
            i += ch.len_utf8();
        }
    }
    word_count(&text)
}

/// Reading time at 230 words per minute, for example "6 min read" or "1 h 5 min read".
pub fn format_read_time(words: u32) -> Option<String> {
    if words == 0 {
        return None;
    }
    let m = words.div_ceil(230).max(1);
    Some(if m < 60 {
        format!("{m} min read")
    } else if m.is_multiple_of(60) {
        format!("{} h read", m / 60)
    } else {
        format!("{} h {} min read", m / 60, m % 60)
    })
}

/// Time left at 230 words per minute: "Less than a minute left", "4 min left", "1 h 5 min left".
pub fn format_time_left(words_left: u32) -> String {
    let m = words_left / 230;
    if m == 0 {
        "Less than a minute left".to_string()
    } else if m < 60 {
        format!("{m} min left")
    } else if m.is_multiple_of(60) {
        format!("{} h left", m / 60)
    } else {
        format!("{} h {} min left", m / 60, m % 60)
    }
}

/// Largest article HTML accepted for processing.
pub const MAX_HTML: usize = 16 * 1024 * 1024;
/// Placeholder link scheme the reader handles to retry one image.
pub const RETRY_SCHEME: &str = "inoink-retry:";

#[derive(Clone, Debug, PartialEq)]
pub enum AssetState {
    /// Downloaded; the value is the final file name inside `assets/` (with extension).
    Done(String),
    Pending,
    Failed,
}

pub type AssetMap = HashMap<String, AssetState>;

/// An image the article refers to.
#[derive(Clone, Debug, PartialEq)]
pub struct AssetRef {
    /// Stable name derived from the absolute URL (no extension).
    pub name: String,
    pub url: String,
}

#[derive(Clone, Debug, Default)]
pub struct ArticleMeta {
    pub title: String,
    pub source: String,
    pub author: String,
    /// Already formatted for display.
    pub date: String,
    pub url: String,
    /// Link label to the cached comments page, for discussion link posts.
    pub discussion: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedArticle {
    /// Complete XHTML document.
    pub html: String,
    /// Plain text for indexing.
    pub text: String,
    /// Every image referenced, in document order, deduplicated.
    pub assets: Vec<AssetRef>,
    /// Number of images shown as placeholders because they are not downloaded.
    pub placeholders: usize,
}

/// Stable asset name for a URL: first 32 hex digits of its SHA-256.
pub fn asset_name(url: &str) -> String {
    crate::util::sha256_hex(url.as_bytes())[..32].to_string()
}

pub use sanitize::prepare_article;

/// Finds where a saved position lives in a new content version. `anchor` is a text snippet from
/// the page the reader was on; the occurrence nearest to `old_offset` wins. Returns `None` when
/// the snippet is gone, in which case callers clamp the old offset to the new document length.
pub fn relocate(new_html: &str, anchor: &str, old_offset: usize) -> Option<usize> {
    let anchor = anchor.trim();
    if anchor.is_empty() {
        return None;
    }
    let mut best: Option<usize> = None;
    let mut from = 0;
    while let Some(i) = new_html[from..].find(anchor) {
        let pos = from + i;
        let better = match best {
            None => true,
            Some(b) => pos.abs_diff(old_offset) < b.abs_diff(old_offset),
        };
        if better {
            best = Some(pos);
        }
        from = pos + anchor.len().max(1);
        if from >= new_html.len() {
            break;
        }
    }
    best
}
