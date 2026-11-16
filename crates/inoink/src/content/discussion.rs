//! Link posts from discussion sites (Hacker News, Lobsters): detection, the public thread
//! APIs, and the comments page. This is an InoInk feature, not an Inoreader API; nothing here
//! sends credentials.

use super::sanitize::{esc, sanitize_fragment};
use super::PreparedArticle;
use crate::error::{Error, Result};
use serde_json::Value;

/// Link scheme the reader handles to switch to the comments page.
pub const COMMENTS_SCHEME: &str = "inoink-comments:";
/// Link scheme the reader handles to switch back to the article.
pub const ARTICLE_SCHEME: &str = "inoink-article:";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Site {
    HackerNews,
    Lobsters,
}

impl Site {
    pub fn as_str(self) -> &'static str {
        match self {
            Site::HackerNews => "hn",
            Site::Lobsters => "lobsters",
        }
    }

    pub fn from_name(s: &str) -> Option<Site> {
        match s {
            "hn" => Some(Site::HackerNews),
            "lobsters" => Some(Site::Lobsters),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Site::HackerNews => "Hacker News",
            Site::Lobsters => "Lobsters",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Discussion {
    pub site: Site,
    pub id: String,
}

impl Discussion {
    /// The human-readable comments page.
    pub fn url(&self) -> String {
        match self.site {
            Site::HackerNews => format!("https://news.ycombinator.com/item?id={}", self.id),
            Site::Lobsters => format!("https://lobste.rs/s/{}", self.id),
        }
    }

    /// The public JSON for the whole thread.
    pub fn api_url(&self) -> String {
        match self.site {
            Site::HackerNews => format!("https://hn.algolia.com/api/v1/items/{}", self.id),
            Site::Lobsters => format!("https://lobste.rs/s/{}.json", self.id),
        }
    }
}

fn id_after(text: &str, marker: &str, ok: impl Fn(char) -> bool) -> Option<String> {
    let at = text.find(marker)? + marker.len();
    let id: String = text[at..].chars().take_while(|c| ok(*c)).collect();
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}

fn detect_in(text: &str) -> Option<Discussion> {
    let hn = id_after(text, "news.ycombinator.com/item?id=", |c| c.is_ascii_digit())
        .map(|id| (text.find("news.ycombinator.com/item?id="), Discussion { site: Site::HackerNews, id }));
    let lo = id_after(text, "lobste.rs/s/", |c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .map(|id| (text.find("lobste.rs/s/"), Discussion { site: Site::Lobsters, id }));
    match (hn, lo) {
        (Some((a, d)), Some((b, e))) => Some(if a <= b { d } else { e }),
        (Some((_, d)), None) | (None, Some((_, d))) => Some(d),
        (None, None) => None,
    }
}

/// Recognises a Hacker News or Lobsters item from its link or summary.
pub fn detect(canonical_url: &str, summary_html: &str) -> Option<Discussion> {
    detect_in(canonical_url).or_else(|| detect_in(summary_html))
}

/// True for the comments page of a discussion site (never extracted as an article).
pub fn is_discussion_url(url: &str) -> bool {
    url.contains("news.ycombinator.com/item") || url.contains("lobste.rs/s/")
}

#[derive(Clone, Debug, PartialEq)]
pub struct Comment {
    pub author: String,
    pub html: String,
    /// When it was posted, in Unix seconds (0 when unknown).
    pub time: i64,
    pub depth: u32,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Thread {
    pub title: String,
    pub article_url: Option<String>,
    pub post_html: Option<String>,
    pub comments: Vec<Comment>,
    pub count: u32,
    /// Who submitted the story (marked OP in the thread).
    pub submitter: Option<String>,
}

fn non_empty(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.trim().is_empty()).map(String::from)
}

fn walk_hn(node: &Value, depth: u32, out: &mut Vec<Comment>, count: &mut u32) {
    let is_comment = node.get("type").and_then(Value::as_str).map(|t| t == "comment").unwrap_or(true);
    let children = node.get("children").and_then(Value::as_array).cloned().unwrap_or_default();
    if is_comment {
        let author = non_empty(&node["author"]);
        let text = non_empty(&node["text"]);
        if author.is_none() && text.is_none() {
            // Deleted: keep a stub only so its replies keep their context.
            if !children.is_empty() {
                out.push(Comment { author: "[deleted]".into(), html: String::new(), time: 0, depth });
            }
        } else {
            *count += 1;
            out.push(Comment {
                author: author.unwrap_or_else(|| "[deleted]".into()),
                html: text.unwrap_or_default(),
                time: node["created_at_i"].as_i64().unwrap_or(0),
                depth,
            });
        }
    }
    let next = if is_comment { depth + 1 } else { depth };
    for c in &children {
        walk_hn(c, next, out, count);
    }
}

/// Parses the thread JSON of `site`.
pub fn parse_thread(site: Site, json: &[u8]) -> Result<Thread> {
    let v: Value = serde_json::from_slice(json).map_err(|_| Error::InvalidRequest("unreadable discussion".into()))?;
    let mut t = Thread::default();
    match site {
        Site::HackerNews => {
            if !v.is_object() {
                return Err(Error::InvalidRequest("unreadable discussion".into()));
            }
            t.title = v["title"].as_str().unwrap_or_default().to_string();
            t.article_url = non_empty(&v["url"]);
            t.post_html = non_empty(&v["text"]);
            t.submitter = non_empty(&v["author"]);
            let mut count = 0;
            for c in v["children"].as_array().cloned().unwrap_or_default() {
                walk_hn(&c, 0, &mut t.comments, &mut count);
            }
            t.count = count;
        }
        Site::Lobsters => {
            if !v.is_object() {
                return Err(Error::InvalidRequest("unreadable discussion".into()));
            }
            t.title = v["title"].as_str().unwrap_or_default().to_string();
            t.article_url = non_empty(&v["url"]);
            t.post_html = non_empty(&v["description"]);
            // A name, or an object with one, depending on the API version.
            t.submitter = non_empty(&v["submitter_user"]).or_else(|| non_empty(&v["submitter_user"]["username"]));
            for c in v["comments"].as_array().cloned().unwrap_or_default() {
                let depth = c["depth"].as_u64().unwrap_or(0) as u32;
                if c["is_deleted"].as_bool().unwrap_or(false) {
                    t.comments.push(Comment { author: "[deleted]".into(), html: String::new(), time: 0, depth });
                } else {
                    t.count += 1;
                    t.comments.push(Comment {
                        author: c["commenting_user"].as_str().unwrap_or_default().to_string(),
                        html: c["comment"].as_str().unwrap_or_default().to_string(),
                        time: c["created_at"].as_str().and_then(parse_iso8601).unwrap_or(0),
                        depth,
                    });
                }
            }
        }
    }
    Ok(t)
}

/// "No comments yet", "1 comment", "42 comments".
pub fn count_text(n: u32) -> String {
    match n {
        0 => "No comments yet".to_string(),
        1 => "1 comment".to_string(),
        n => format!("{n} comments"),
    }
}

/// Unix seconds of an ISO 8601 time like `2026-11-11T12:16:50.200-06:00` or `...Z`.
fn parse_iso8601(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !(b[10] == b'T' || b[10] == b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (num(0, 4)?, num(5, 7)?, num(8, 10)?, num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    let mut rest = &s[19..];
    if let Some(r) = rest.strip_prefix('.') {
        let digits = r.bytes().take_while(u8::is_ascii_digit).count();
        rest = &r[digits..];
    }
    let offset = match rest.as_bytes() {
        [] | [b'Z'] => 0,
        [sign @ (b'+' | b'-'), _, _, b':', _, _] | [sign @ (b'+' | b'-'), _, _, _, _] => {
            let hh = rest.get(1..3)?.parse::<i64>().ok()?;
            let mm = rest.get(rest.len() - 2..)?.parse::<i64>().ok()?;
            let v = hh * 3600 + mm * 60;
            if *sign == b'-' {
                -v
            } else {
                v
            }
        }
        _ => return None,
    };
    Some(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + se - offset)
}

/// Days since 1970-01-01 of a civil date (the inverse of `cache::civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// How long before `now` a comment was posted, briefly: "now", "5 min", "3 h", "2 d", "4 mo", "1 y".
pub fn age_text(secs: i64, now: i64) -> String {
    if secs <= 0 {
        return String::new();
    }
    let d = now - secs;
    match d {
        d if d < 60 => "now".to_string(),
        d if d < 3600 => format!("{} min", d / 60),
        d if d < 86_400 => format!("{} h", d / 3600),
        d if d < 30 * 86_400 => format!("{} d", d / 86_400),
        d if d < 365 * 86_400 => format!("{} mo", d / (30 * 86_400)),
        d => format!("{} y", d / (365 * 86_400)),
    }
}

/// The comments page as a complete XHTML document. `now` dates the ages shown ("3 h"): the page
/// is generated when the comments are fetched, so they are as of then.
pub fn render_comments(thread: &Thread, d: &Discussion, title: &str, now: i64) -> PreparedArticle {
    let mut html = String::new();
    html.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>");
    esc(title, &mut html);
    html.push_str("</title></head><body><h1>Comments: ");
    esc(title, &mut html);
    html.push_str("</h1><p class=\"inoink-meta\">");
    let summary = if thread.count == 0 {
        count_text(0)
    } else {
        format!("{} on {} \u{b7} as of {}", count_text(thread.count), d.site.label(), crate::cache::format_date(now))
    };
    esc(&summary, &mut html);
    html.push_str("</p><p class=\"inoink-switch\"><a href=\"");
    html.push_str(ARTICLE_SCHEME);
    html.push_str("\">Back to the article</a></p>");
    let mut text = String::new();
    let threads = thread.comments.iter().filter(|c| c.depth == 0).count();
    // Replies under each top-level comment: everything up to the next one, minus deleted stubs.
    let mut reply_counts: Vec<usize> = Vec::new();
    for c in &thread.comments {
        if c.depth == 0 {
            reply_counts.push(0);
        } else if let Some(k) = reply_counts.last_mut() {
            if c.author != "[deleted]" {
                *k += 1;
            }
        }
    }
    let mut n = 0;
    // `<div class="inoink-replies">` containers open now: one per indent level, two at most.
    let mut open = 0usize;
    let mut thread_open = false;
    // The author at each depth so far: a deep reply names the comment it answers.
    let mut parents: Vec<String> = Vec::new();
    for c in &thread.comments {
        let (body, body_text) = sanitize_fragment(&c.html, Some(&d.url()));
        let depth = c.depth as usize;
        parents.truncate(depth);
        let parent = if depth > 0 { parents.last().cloned() } else { None };
        parents.resize(depth, String::new());
        parents.push(c.author.clone());
        let visual = depth.min(2);
        while open > visual || (depth == 0 && open > 0) {
            html.push_str("</div>");
            open -= 1;
        }
        if depth == 0 {
            if thread_open {
                html.push_str("</div>");
            }
            n += 1;
            html.push_str(&format!("<p class=\"inoink-sep\" id=\"inoink-t{n}\">\u{2042}</p><div class=\"inoink-thread\">"));
            thread_open = true;
        }
        while open < visual {
            html.push_str("<div class=\"inoink-replies\">");
            open += 1;
        }
        html.push_str("<div class=\"inoink-c\"><p class=\"inoink-cm\">");
        if c.author == "[deleted]" && c.html.is_empty() {
            html.push_str("<i>deleted</i>");
        } else {
            html.push_str("<b>");
            esc(&c.author, &mut html);
            html.push_str("</b>");
            if thread.submitter.as_deref().is_some_and(|s| s == c.author) {
                html.push_str(" OP");
            }
            if depth >= 3 {
                if let Some(p) = parent {
                    esc(&format!(" \u{21b3} {p}"), &mut html);
                }
            }
            let age = age_text(c.time, now);
            if !age.is_empty() {
                esc(&format!(" \u{b7} {age}"), &mut html);
            }
            if depth == 0 {
                match reply_counts[n - 1] {
                    0 => {}
                    1 => html.push_str(" \u{b7} 1 reply"),
                    k => html.push_str(&format!(" \u{b7} {k} replies")),
                }
                if n < threads {
                    html.push_str(&format!(" \u{b7} <a href=\"#inoink-t{}\">Skip \u{203a}</a>", n + 1));
                }
            }
        }
        html.push_str("</p>");
        html.push_str(&body);
        html.push_str("</div>");
        text.push_str(&c.author);
        text.push(' ');
        text.push_str(&body_text);
        text.push('\n');
    }
    while open > 0 {
        html.push_str("</div>");
        open -= 1;
    }
    if thread_open {
        html.push_str("</div>");
    }
    html.push_str("</body></html>\n");
    PreparedArticle { html, text: text.trim().to_string(), assets: Vec::new(), placeholders: 0 }
}
