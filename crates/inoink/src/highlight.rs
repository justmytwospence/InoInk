//! Highlights and annotations: Inoreader's read-only annotations and quote anchoring.
//!
//! A highlight is identified by the text it covers plus a little context (a [`TextQuote`]), not
//! by a position: feed text, extracted page text and Inoreader's own rendering all differ in
//! markup, hyphenation and spacing, so positions never carry over between them.

use crate::model::ArticleId;
use markup5ever_rcdom::{Handle, NodeData};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};

/// One annotation as returned by Inoreader with `annotations=1`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteAnnotation {
    pub id: i64,
    pub start: i64,
    pub end: i64,
    pub text: String,
    pub note: String,
    pub added_on: i64,
}

fn num(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// The annotations of an article object, or `None` when the object carries no `annotations`
/// array (it was fetched without them, so nothing may be concluded). Entries without an id
/// are skipped.
pub fn parse_annotations(item: &Value) -> Option<Vec<RemoteAnnotation>> {
    let arr = item["annotations"].as_array()?;
    Some(
        arr.iter()
            .filter_map(|a| {
                Some(RemoteAnnotation {
                    id: num(&a["id"])?,
                    start: num(&a["start"]).unwrap_or(0),
                    end: num(&a["end"]).unwrap_or(0),
                    text: a["text"].as_str().unwrap_or("").to_string(),
                    note: a["note"].as_str().unwrap_or("").to_string(),
                    added_on: num(&a["added_on"]).unwrap_or(0),
                })
            })
            .collect(),
    )
}

/// Characters of context kept before and after a quote taken from text.
pub const CONTEXT_CHARS: usize = 64;
/// Words of context kept before and after a quote taken from reader words.
pub const CONTEXT_WORDS: usize = 8;

/// The text of a highlight with what surrounds it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextQuote {
    pub exact: String,
    pub prefix: String,
    pub suffix: String,
}

fn collect_text(node: &Handle, out: &mut String, depth: usize) {
    if depth > 256 {
        return;
    }
    match &node.data {
        NodeData::Text { contents } => out.push_str(&contents.borrow()),
        NodeData::Element { name, .. } if matches!(&*name.local, "script" | "style" | "noscript" | "template") => return,
        _ => {}
    }
    for child in node.children.borrow().iter() {
        collect_text(child, out, depth + 1);
    }
}

/// The text of an HTML fragment as a browser's `textContent` gives it: every text node in
/// document order with entities decoded, nothing added or collapsed.
pub fn text_content(html: &str) -> String {
    let dom = crate::content::sanitize::parse(html);
    let mut out = String::new();
    collect_text(&dom.document, &mut out, 0);
    out
}

/// Lowercase alphanumerics only: hyphenation, soft hyphens, punctuation and spacing differ
/// between renderings of the same text.
fn norm(s: &str) -> Vec<char> {
    s.chars().flat_map(char::to_lowercase).filter(|c| c.is_alphanumeric()).collect()
}

/// The quote of an Inoreader annotation. Its `start`/`end` count UTF-16 units of the article's
/// rendered text; they are used only to cut context out of `summary_html`, and only when the cut
/// text agrees with the annotation's own text. Otherwise there is no context.
pub fn quote_from_inoreader(summary_html: &str, a: &RemoteAnnotation) -> TextQuote {
    let exact = a.text.trim().to_string();
    let mut q = TextQuote { exact, ..Default::default() };
    let units: Vec<u16> = text_content(summary_html).encode_utf16().collect();
    if a.start >= 0 && a.start < a.end && (a.end as usize) <= units.len() {
        let (s, e) = (a.start as usize, a.end as usize);
        if norm(&String::from_utf16_lossy(&units[s..e])) == norm(&q.exact) {
            let before = String::from_utf16_lossy(&units[..s]);
            let after = String::from_utf16_lossy(&units[e..]);
            let mut p: Vec<char> = before.chars().rev().take(CONTEXT_CHARS).collect();
            p.reverse();
            q.prefix = p.into_iter().collect();
            q.suffix = after.chars().take(CONTEXT_CHARS).collect();
        }
    }
    q
}

/// The quote of the words `first..=last` (indices into `words`) with their neighbours.
pub fn quote_from_words(words: &[&str], first: usize, last: usize) -> TextQuote {
    if words.is_empty() || first > last || first >= words.len() {
        return TextQuote::default();
    }
    let last = last.min(words.len() - 1);
    TextQuote {
        exact: words[first..=last].join(" "),
        prefix: words[first.saturating_sub(CONTEXT_WORDS)..first].join(" "),
        suffix: words[last + 1..(last + 1 + CONTEXT_WORDS).min(words.len())].join(" "),
    }
}

/// Finds `quote` among `words` (text and location, in document order) and returns the locations
/// of the first and last word it covers. Among repeats the best agreement with the quote's
/// context wins, then the nearest to `hint`, then the earliest. `None` when it is not there.
pub fn locate(words: &[(&str, usize)], quote: &TextQuote, hint: Option<usize>) -> Option<(usize, usize)> {
    let needle = norm(&quote.exact);
    let n = needle.len();
    if n == 0 {
        return None;
    }
    let mut hay: Vec<char> = Vec::new();
    let mut owner: Vec<usize> = Vec::new();
    for (i, (w, _)) in words.iter().enumerate() {
        for c in norm(w) {
            hay.push(c);
            owner.push(i);
        }
    }
    if hay.len() < n {
        return None;
    }
    let mut found: Vec<usize> = (0..=hay.len() - n).filter(|&i| hay[i..i + n] == needle[..]).collect();
    // A selection made on words starts and ends on word edges; prefer matches that do.
    let aligned: Vec<usize> = found
        .iter()
        .copied()
        .filter(|&i| (i == 0 || owner[i - 1] != owner[i]) && (i + n == hay.len() || owner[i + n] != owner[i + n - 1]))
        .collect();
    if !aligned.is_empty() {
        found = aligned;
    }
    let prefix = norm(&quote.prefix);
    let suffix = norm(&quote.suffix);
    let score = |i: usize| -> usize {
        let before = (0..prefix.len().min(i)).take_while(|&k| hay[i - 1 - k] == prefix[prefix.len() - 1 - k]).count();
        let after = (0..suffix.len().min(hay.len() - i - n)).take_while(|&k| hay[i + n + k] == suffix[k]).count();
        before + after
    };
    let dist = |i: usize| hint.map_or(0, |h| words[owner[i]].1.abs_diff(h));
    let best = found.into_iter().min_by(|&a, &b| score(b).cmp(&score(a)).then(dist(a).cmp(&dist(b))).then(a.cmp(&b)))?;
    Some((words[owner[best]].1, words[owner[best + n - 1]].1))
}

/// A random 128-bit identifier as 32 lowercase hex digits.
pub fn new_uid() -> String {
    use std::io::Read;
    let mut buf = [0u8; 16];
    if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf)).is_ok() {
        return buf.iter().map(|b| format!("{b:02x}")).collect();
    }
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let seed = format!("{}:{}:{}", nanos, std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
    crate::util::sha256_hex(seed.as_bytes())[..32].to_string()
}

/// `YYYY-MM-DDTHH:MM:SSZ` for a Unix time.
pub fn rfc3339_utc(secs: i64) -> String {
    let (y, m, d) = crate::cache::civil(secs);
    let t = secs.rem_euclid(86400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", t / 3600, (t % 3600) / 60, t % 60)
}

/// A highlight made on this device. Deleted ones are kept (`deleted`) so a later sync can tell
/// the other side; `modified` says what changed since then.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Highlight {
    pub uid: String,
    pub article: ArticleId,
    pub quote: TextQuote,
    pub note: String,
    /// Content version the locations below belong to.
    pub content_hash: String,
    pub start_loc: i64,
    pub end_loc: i64,
    pub created: i64,
    pub modified: i64,
    pub deleted: bool,
}

/// A highlight with what a reading-notes service needs to know about its article.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct HighlightExport {
    pub uid: String,
    /// The article's long id (`tag:google.com,2005:reader/item/...`).
    pub article: String,
    pub text: String,
    pub note: String,
    pub title: String,
    pub author: String,
    pub source_url: String,
    pub feed_title: String,
    pub highlighted_at: i64,
    pub modified: i64,
}

/// Highlights as Markdown, grouped by article in the order given.
pub fn to_markdown(list: &[HighlightExport]) -> String {
    let mut out = String::from("# InoInk highlights\n\n");
    let mut current: Option<&str> = None;
    for h in list {
        if current != Some(h.article.as_str()) {
            current = Some(h.article.as_str());
            out.push_str(&format!("## {}\n\n{}", h.title, h.feed_title));
            if !h.author.is_empty() {
                out.push_str(&format!(" \u{b7} {}", h.author));
            }
            out.push_str(&format!("\n{}\n\n", h.source_url));
        }
        out.push_str(&format!("> {}\n\n", h.text));
        if !h.note.is_empty() {
            out.push_str(&format!("{}\n\n", h.note));
        }
    }
    out
}

/// Highlights in the shape of Readwise's "create highlights" request. Nothing is sent.
pub fn to_readwise_json(list: &[HighlightExport]) -> Value {
    const LIMIT: usize = 8191;
    let cut = |s: &str| s.chars().take(LIMIT).collect::<String>();
    let items: Vec<Value> = list
        .iter()
        .map(|h| {
            let mut o = serde_json::json!({
                "text": cut(&h.text),
                "title": h.title,
                "source_url": h.source_url,
                "source_type": "inoink",
                "category": "articles",
                "highlighted_at": rfc3339_utc(h.highlighted_at),
            });
            if !h.author.is_empty() {
                o["author"] = Value::String(h.author.clone());
            }
            if !h.note.is_empty() {
                o["note"] = Value::String(cut(&h.note));
            }
            o
        })
        .collect();
    serde_json::json!({ "highlights": items })
}
