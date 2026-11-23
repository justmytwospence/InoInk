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

/// How a highlight made on the device is drawn. Inoreader's annotations carry no style, so it is
/// kept on the device only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HighlightStyle {
    /// The words are shaded (tinted on colour screens).
    #[default]
    Shaded,
    Underline,
    /// A dotted underline.
    Dotted,
}

impl HighlightStyle {
    pub const ALL: [HighlightStyle; 3] = [HighlightStyle::Shaded, HighlightStyle::Underline, HighlightStyle::Dotted];

    pub fn as_str(self) -> &'static str {
        match self {
            HighlightStyle::Shaded => "shaded",
            HighlightStyle::Underline => "underline",
            HighlightStyle::Dotted => "dotted",
        }
    }

    /// The style stored as `s`; anything unknown is shaded.
    pub fn parse(s: &str) -> HighlightStyle {
        HighlightStyle::ALL.into_iter().find(|v| v.as_str() == s).unwrap_or_default()
    }

    pub fn label(self) -> &'static str {
        match self {
            HighlightStyle::Shaded => "Shading",
            HighlightStyle::Underline => "Underline",
            HighlightStyle::Dotted => "Dotted underline",
        }
    }
}

/// A highlight's colour. Grayscale screens draw every colour as gray.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HighlightColor {
    #[default]
    Yellow,
    Green,
    Blue,
    Pink,
    Gray,
}

impl HighlightColor {
    pub const ALL: [HighlightColor; 5] =
        [HighlightColor::Yellow, HighlightColor::Green, HighlightColor::Blue, HighlightColor::Pink, HighlightColor::Gray];

    pub fn as_str(self) -> &'static str {
        match self {
            HighlightColor::Yellow => "yellow",
            HighlightColor::Green => "green",
            HighlightColor::Blue => "blue",
            HighlightColor::Pink => "pink",
            HighlightColor::Gray => "gray",
        }
    }

    /// The colour stored as `s`; anything unknown is gray.
    pub fn parse(s: &str) -> HighlightColor {
        HighlightColor::ALL.into_iter().find(|v| v.as_str() == s).unwrap_or(HighlightColor::Gray)
    }

    pub fn label(self) -> &'static str {
        match self {
            HighlightColor::Yellow => "Yellow",
            HighlightColor::Green => "Green",
            HighlightColor::Blue => "Blue",
            HighlightColor::Pink => "Pink",
            HighlightColor::Gray => "Gray",
        }
    }

    /// The tint multiplied into shaded text: saturated enough to show on a colour e-ink screen.
    pub fn tint(self) -> [u8; 3] {
        match self {
            HighlightColor::Yellow => [0xF5, 0xE0, 0x5C],
            HighlightColor::Green => [0xA8, 0xDC, 0x8C],
            HighlightColor::Blue => [0x96, 0xC8, 0xEC],
            HighlightColor::Pink => [0xF2, 0xA6, 0xC4],
            HighlightColor::Gray => [0xDD, 0xDD, 0xDD],
        }
    }

    /// The colour of underlines: the tint darkened so a thin line still reads; gray is black.
    pub fn ink(self) -> [u8; 3] {
        match self {
            HighlightColor::Gray => [0, 0, 0],
            c => c.tint().map(|v| (v as u16 * 11 / 20) as u8),
        }
    }

    pub fn css(self) -> String {
        let [r, g, b] = self.tint();
        format!("#{r:02x}{g:02x}{b:02x}")
    }
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
    pub style: HighlightStyle,
    pub color: HighlightColor,
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
    pub style: HighlightStyle,
    pub color: HighlightColor,
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

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Paragraphs of `s` (separated by blank lines), each line break kept.
fn html_paragraphs(s: &str) -> String {
    s.replace("\r\n", "\n")
        .split("\n\n")
        .filter(|p| !p.trim().is_empty())
        .map(|p| format!("<p>{}</p>", html_escape(p.trim()).replace('\n', "<br>")))
        .collect()
}

/// Highlights as one self-contained HTML page, grouped by article in the order given, each
/// drawn in its style and colour. `exported_at` is a Unix time.
pub fn to_html(list: &[HighlightExport], exported_at: i64) -> String {
    let mut out = String::from(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>InoInk highlights</title><style>\
         body{font-family:Georgia,serif;max-width:42em;margin:2em auto;padding:0 1em;line-height:1.5;color:#222}\
         h2{margin-top:2em;font-size:1.3em}.meta{color:#666;font-size:.9em}.meta a{color:inherit}\
         blockquote{margin:1em 0;padding:0}blockquote p{margin:.4em 0}\
         .shaded p span{background:var(--mark);box-decoration-break:clone;-webkit-box-decoration-break:clone}\
         .underline p span{text-decoration:underline 2px var(--ink);text-underline-offset:3px}\
         .dotted p span{text-decoration:underline dotted 2px var(--ink);text-underline-offset:3px}\
         .note{margin:.2em 0 1.4em 1em;padding-left:.8em;border-left:3px solid #ccc;color:#444}\
         footer{margin-top:3em;color:#888;font-size:.85em}</style></head><body>\n",
    );
    let notes = list.iter().filter(|h| !h.note.is_empty()).count();
    out.push_str(&format!(
        "<h1>InoInk highlights</h1>\n<p class=\"meta\">{} highlight{} \u{b7} {} note{} \u{b7} exported {}</p>\n",
        list.len(),
        if list.len() == 1 { "" } else { "s" },
        notes,
        if notes == 1 { "" } else { "s" },
        rfc3339_utc(exported_at)
    ));
    let mut current: Option<&str> = None;
    for h in list {
        if current != Some(h.article.as_str()) {
            current = Some(h.article.as_str());
            out.push_str(&format!("<h2>{}</h2>\n<p class=\"meta\">{}", html_escape(&h.title), html_escape(&h.feed_title)));
            if !h.author.is_empty() {
                out.push_str(&format!(" \u{b7} {}", html_escape(&h.author)));
            }
            if h.source_url.starts_with("https://") || h.source_url.starts_with("http://") {
                out.push_str(&format!("<br><a href=\"{0}\">{0}</a>", html_escape(&h.source_url)));
            }
            out.push_str("</p>\n");
        }
        let [r, g, b] = h.color.ink();
        let paragraphs = html_paragraphs(&h.text).replace("<p>", "<p><span>").replace("</p>", "</span></p>");
        out.push_str(&format!(
            "<blockquote class=\"{}\" data-color=\"{}\" style=\"--mark:{};--ink:#{r:02x}{g:02x}{b:02x}\">{}</blockquote>\n",
            h.style.as_str(),
            h.color.as_str(),
            h.color.css(),
            paragraphs
        ));
        if !h.note.is_empty() {
            out.push_str(&format!("<div class=\"note\">{}</div>\n", html_paragraphs(&h.note)));
        }
    }
    out.push_str("<footer>Exported from InoInk.</footer>\n</body></html>\n");
    out
}

/// The files the device exports highlights to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Markdown,
    Html,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Markdown => "md",
            ExportFormat::Html => "html",
        }
    }

    /// The file's text, exported at `now` (Unix time).
    pub fn render(self, list: &[HighlightExport], now: i64) -> String {
        match self {
            ExportFormat::Markdown => to_markdown(list),
            ExportFormat::Html => to_html(list, now),
        }
    }

    /// `inoink-highlights-2027-01-15-080000.md`: sorts by date, unique to the second.
    pub fn file_name(self, now: i64) -> String {
        let stamp: String = rfc3339_utc(now).chars().filter(|c| c.is_ascii_digit() || *c == '-' || *c == 'T').collect();
        let stamp = stamp.replacen('T', "-", 1);
        format!("inoink-highlights-{stamp}.{}", self.extension())
    }
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
