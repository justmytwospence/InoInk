//! HTML pages for article details, summaries and annotations, shown in the reader.

use inoink::model::*;
use inoink::store::{ArticleRecord, Store};

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// What kind of content is cached, stated accurately: the API gives no excerpt flag.
pub fn content_kind(store: &Store, id: ArticleId) -> &'static str {
    match store.latest_version(id) {
        Ok(Some(v)) if v.raw_hash.starts_with("extract:") => "Original page (loaded by InoInk)",
        Ok(Some(_)) => "Feed content (may be shortened by the publisher)",
        _ => "Not downloaded",
    }
}

fn state_text(s: DownloadState) -> &'static str {
    match s {
        DownloadState::Complete => "Text and images downloaded",
        DownloadState::TextReady => "Text downloaded, images pending",
        DownloadState::MissingImages => "Text downloaded, some images unavailable",
        DownloadState::Failed => "Download failed",
        DownloadState::NotDownloaded => "Not downloaded",
    }
}

/// Details, summaries and annotations of one article as a standalone XHTML page.
pub fn details_html(store: &Store, id: ArticleId, rec: &ArticleRecord) -> String {
    let mut h = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>Details</title></head><body>");
    h.push_str(&format!("<h1>{}</h1>", esc(&rec.title)));
    let row = |k: &str, v: &str| format!("<p><b>{}</b><br/>{}</p>", esc(k), esc(v));
    h.push_str(&row("Source", &rec.origin_title));
    if !rec.author.is_empty() {
        h.push_str(&row("Author", &rec.author));
    }
    h.push_str(&row("Published", &inoink::cache::format_date(rec.published)));
    if !rec.canonical_url.is_empty() {
        h.push_str(&row("Address", &rec.canonical_url));
    }
    h.push_str(&row("Content", content_kind(store, id)));
    h.push_str(&row("Offline", state_text(store.download_state(id).unwrap_or(DownloadState::NotDownloaded))));
    let flag = |m: Membership| store.effective(id, &m).unwrap_or(false);
    let states = [(flag(Membership::Read), "Read"), (flag(Membership::Starred), "Starred"), (flag(Membership::Like), "Liked"), (flag(Membership::Broadcast), "Broadcast")];
    let on: Vec<&str> = states.iter().filter(|(v, _)| *v).map(|(_, n)| *n).collect();
    let state = if on.is_empty() { "Unread".to_string() } else { on.join(", ") };
    h.push_str(&row("State", &state));
    if let Ok(pending) = store.pending_for(id) {
        if !pending.is_empty() {
            h.push_str(&row("Waiting to send", &format!("{} change(s)", pending.len())));
        }
    }
    let raw: serde_json::Value = serde_json::from_str(&rec.raw_json).unwrap_or_default();
    if let Some(sums) = raw["summaries"].as_array().filter(|a| !a.is_empty()) {
        h.push_str("<h2>Summaries</h2>");
        for s in sums {
            let name = s["prompt_name"].as_str().unwrap_or("Summary");
            // Summary HTML from the API is sanitized like article content.
            let body = inoink::content::prepare_article(s["summary"].as_str().unwrap_or(""), None, &Default::default(), &Default::default());
            h.push_str(&format!("<h3>{}</h3><p>{}</p>", esc(name), esc(&body.text)));
        }
    }
    let mine = store.highlights(id).unwrap_or_default();
    if !mine.is_empty() {
        h.push_str("<h2>Your highlights</h2>");
        for m in &mine {
            h.push_str(&format!("<blockquote><p>{}</p></blockquote>", esc(&m.quote.exact)));
            if !m.note.is_empty() {
                h.push_str(&format!("<p><i>{}</i></p>", esc(&m.note)));
            }
        }
        h.push_str("<p><small>Saved on this Kobo.</small></p>");
    }
    let ann = store.remote_annotations(id).unwrap_or_default();
    if !ann.is_empty() {
        h.push_str("<h2>Inoreader annotations</h2>");
        for a in &ann {
            if a.text.trim().is_empty() {
                h.push_str("<p><b>Note on the article</b></p>");
            } else {
                h.push_str(&format!("<blockquote><p>{}</p></blockquote>", esc(&a.text)));
            }
            if !a.note.is_empty() {
                h.push_str(&format!("<p><i>{}</i></p>", esc(&a.note)));
            }
        }
        h.push_str("<p><small>Made on the Inoreader website or apps; shown read-only.</small></p>");
    }
    h.push_str("</body></html>");
    h
}
