//! Content pipeline: sanitize API HTML into XHTML that Plato's engine renders, resolve and
//! download images, and keep every file under a rooted cache directory.

pub mod extract;
pub mod images;
pub mod sanitize;

use std::collections::HashMap;

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
