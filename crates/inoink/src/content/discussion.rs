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
    pub date: String,
    pub depth: u32,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Thread {
    pub title: String,
    pub article_url: Option<String>,
    pub post_html: Option<String>,
    pub comments: Vec<Comment>,
    pub count: u32,
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
                out.push(Comment { author: "[deleted]".into(), html: String::new(), date: String::new(), depth });
            }
        } else {
            *count += 1;
            out.push(Comment {
                author: author.unwrap_or_else(|| "[deleted]".into()),
                html: text.unwrap_or_default(),
                date: node["created_at_i"].as_i64().map(crate::cache::format_date).unwrap_or_default(),
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
            for c in v["comments"].as_array().cloned().unwrap_or_default() {
                let depth = c["depth"].as_u64().unwrap_or(0) as u32;
                if c["is_deleted"].as_bool().unwrap_or(false) {
                    t.comments.push(Comment { author: "[deleted]".into(), html: String::new(), date: String::new(), depth });
                } else {
                    t.count += 1;
                    t.comments.push(Comment {
                        author: c["commenting_user"].as_str().unwrap_or_default().to_string(),
                        html: c["comment"].as_str().unwrap_or_default().to_string(),
                        date: c["created_at"].as_str().map(|d| d.chars().take(10).collect()).unwrap_or_default(),
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

/// The comments page as a complete XHTML document.
pub fn render_comments(thread: &Thread, d: &Discussion, title: &str) -> PreparedArticle {
    let mut html = String::new();
    html.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>");
    esc(title, &mut html);
    html.push_str("</title></head><body><h1>Comments: ");
    esc(title, &mut html);
    html.push_str("</h1><p class=\"inoink-meta\">");
    let summary = if thread.count == 0 { count_text(0) } else { format!("{} on {}", count_text(thread.count), d.site.label()) };
    esc(&summary, &mut html);
    html.push_str("</p><p class=\"inoink-switch\"><a href=\"");
    html.push_str(ARTICLE_SCHEME);
    html.push_str("\">Back to the article</a></p><hr/>");
    let mut text = String::new();
    for c in &thread.comments {
        let (body, body_text) = sanitize_fragment(&c.html, Some(&d.url()));
        html.push_str(&format!("<div class=\"inoink-comment inoink-d{}\"><p class=\"inoink-cmeta\">", c.depth.min(5)));
        let meta = if c.date.is_empty() { c.author.clone() } else { format!("{} \u{b7} {}", c.author, c.date) };
        esc(&meta, &mut html);
        html.push_str("</p>");
        html.push_str(&body);
        html.push_str("</div>");
        text.push_str(&c.author);
        text.push(' ');
        text.push_str(&body_text);
        text.push('\n');
    }
    html.push_str("</body></html>\n");
    PreparedArticle { html, text: text.trim().to_string(), assets: Vec::new(), placeholders: 0 }
}
