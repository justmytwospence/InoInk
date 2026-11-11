use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const LONG_ID_PREFIX: &str = "tag:google.com,2005:reader/item/";
pub const STATE_READ: &str = "user/-/state/com.google/read";
pub const STATE_STARRED: &str = "user/-/state/com.google/starred";
pub const STATE_LIKE: &str = "user/-/state/com.google/like";
pub const STATE_BROADCAST: &str = "user/-/state/com.google/broadcast";
pub const STATE_SAVED: &str = "user/-/state/com.google/saved-web-pages";
pub const STATE_ANNOTATED: &str = "user/-/state/com.google/annotated";
pub const STATE_READING_LIST: &str = "user/-/state/com.google/reading-list";
pub const STATE_ROOT: &str = "user/-/state/com.google/root";

/// An article identifier: 64 bits, written as signed decimal (short form) or as 16 zero-padded
/// hexadecimal digits behind [`LONG_ID_PREFIX`] (long form).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ArticleId(pub u64);

impl ArticleId {
    pub fn from_short(s: &str) -> Result<ArticleId> {
        s.trim()
            .parse::<i64>()
            .map(|n| ArticleId(n as u64))
            .map_err(|_| Error::Data(format!("bad short article id {s:?}")))
    }

    pub fn from_long(s: &str) -> Result<ArticleId> {
        let hex = s
            .strip_prefix(LONG_ID_PREFIX)
            .ok_or_else(|| Error::Data(format!("bad long article id {s:?}")))?;
        if hex.len() != 16 {
            return Err(Error::Data(format!("bad long article id {s:?}")));
        }
        u64::from_str_radix(hex, 16)
            .map(ArticleId)
            .map_err(|_| Error::Data(format!("bad long article id {s:?}")))
    }

    /// Accepts either wire form.
    pub fn parse(s: &str) -> Result<ArticleId> {
        if s.starts_with(LONG_ID_PREFIX) {
            Self::from_long(s)
        } else {
            Self::from_short(s)
        }
    }

    pub fn to_short(self) -> String {
        (self.0 as i64).to_string()
    }

    pub fn to_long(self) -> String {
        format!("{LONG_ID_PREFIX}{:016x}", self.0)
    }

    /// Storage representation (SQLite integers are signed).
    pub fn to_db(self) -> i64 {
        self.0 as i64
    }

    pub fn from_db(n: i64) -> ArticleId {
        ArticleId(n as u64)
    }
}

impl fmt::Display for ArticleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_short())
    }
}

/// An Inoreader stream identifier such as `feed/https://example.org/rss` or
/// `user/-/label/Tech`. The user id is always normalized to `-`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct StreamId(pub String);

impl StreamId {
    pub fn new(s: &str) -> StreamId {
        StreamId(normalize_user(s))
    }
    pub fn feed(url: &str) -> StreamId {
        StreamId(format!("feed/{url}"))
    }
    pub fn label(name: &str) -> StreamId {
        StreamId(format!("user/-/label/{name}"))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn is_feed(&self) -> bool {
        self.0.starts_with("feed/")
    }
    pub fn label_name(&self) -> Option<&str> {
        self.0.strip_prefix("user/-/label/")
    }
}

/// Replaces `user/<digits>/` with `user/-/`.
pub fn normalize_user(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("user/") {
        if let Some((id, tail)) = rest.split_once('/') {
            if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) {
                return format!("user/-/{tail}");
            }
        }
    }
    s.to_string()
}

/// A per-article state that can be set or cleared independently.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Membership {
    Read,
    Starred,
    Like,
    Broadcast,
    /// A user tag, by bare name (no `user/-/label/` prefix).
    Label(String),
}

impl Membership {
    /// Stable key used in the database.
    pub fn key(&self) -> String {
        match self {
            Membership::Read => "read".into(),
            Membership::Starred => "starred".into(),
            Membership::Like => "like".into(),
            Membership::Broadcast => "broadcast".into(),
            Membership::Label(n) => format!("label:{n}"),
        }
    }

    pub fn from_key(key: &str) -> Result<Membership> {
        Ok(match key {
            "read" => Membership::Read,
            "starred" => Membership::Starred,
            "like" => Membership::Like,
            "broadcast" => Membership::Broadcast,
            k => match k.strip_prefix("label:") {
                Some(n) if !n.is_empty() => Membership::Label(n.to_string()),
                _ => return Err(Error::Data(format!("bad membership key {k:?}"))),
            },
        })
    }

    /// The tag string sent in `edit-tag` `a`/`r`.
    pub fn tag_id(&self) -> String {
        match self {
            Membership::Read => STATE_READ.into(),
            Membership::Starred => STATE_STARRED.into(),
            Membership::Like => STATE_LIKE.into(),
            Membership::Broadcast => STATE_BROADCAST.into(),
            Membership::Label(n) => format!("user/-/label/{n}"),
        }
    }

    /// Recognizes a category/tag string from an article object.
    pub fn from_tag_id(tag: &str) -> Option<Membership> {
        let t = normalize_user(tag);
        match t.as_str() {
            STATE_READ => Some(Membership::Read),
            STATE_STARRED => Some(Membership::Starred),
            STATE_LIKE => Some(Membership::Like),
            STATE_BROADCAST => Some(Membership::Broadcast),
            _ => t.strip_prefix("user/-/label/").map(|n| Membership::Label(n.to_string())),
        }
    }
}

/// What the user is looking at.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Collection {
    Starred,
    Unread,
    All,
    Feed(StreamId),
    Folder(String),
    Tag(String),
    SavedPages,
    Annotated,
    Liked,
    Broadcast,
    /// An Active Search (monitoring) stream.
    Monitoring(StreamId),
}

impl Collection {
    /// Stream id to enumerate on the server for this collection.
    pub fn stream_id(&self) -> StreamId {
        match self {
            Collection::Starred => StreamId::new(STATE_STARRED),
            Collection::Unread | Collection::All => StreamId::new(STATE_READING_LIST),
            Collection::Feed(s) | Collection::Monitoring(s) => s.clone(),
            Collection::Folder(n) | Collection::Tag(n) => StreamId::label(n),
            Collection::SavedPages => StreamId::new(STATE_SAVED),
            Collection::Annotated => StreamId::new(STATE_ANNOTATED),
            Collection::Liked => StreamId::new(STATE_LIKE),
            Collection::Broadcast => StreamId::new(STATE_BROADCAST),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DownloadState {
    NotDownloaded,
    /// Text is cached and readable; images not (all) present yet.
    TextReady,
    /// Text and every image are cached.
    Complete,
    /// Text is cached; some images permanently failed (retryable placeholders shown).
    MissingImages,
    Failed,
}

impl DownloadState {
    pub fn as_str(self) -> &'static str {
        match self {
            DownloadState::NotDownloaded => "not_downloaded",
            DownloadState::TextReady => "text_ready",
            DownloadState::Complete => "complete",
            DownloadState::MissingImages => "missing_images",
            DownloadState::Failed => "failed",
        }
    }
    pub fn parse(s: &str) -> DownloadState {
        match s {
            "text_ready" => DownloadState::TextReady,
            "complete" => DownloadState::Complete,
            "missing_images" => DownloadState::MissingImages,
            "failed" => DownloadState::Failed,
            _ => DownloadState::NotDownloaded,
        }
    }
    /// True when the article can be read without a network.
    pub fn readable_offline(self) -> bool {
        matches!(self, DownloadState::TextReady | DownloadState::Complete | DownloadState::MissingImages)
    }
}

/// A list row.
#[derive(Clone, Debug, PartialEq)]
pub struct ArticleSummary {
    pub id: ArticleId,
    pub title: String,
    pub source: String,
    pub published: i64,
    pub timestamp_usec: i64,
    pub read: bool,
    pub starred: bool,
    pub download: DownloadState,
}

#[derive(Clone, Debug, Default)]
pub struct ArticleQuery {
    pub collection: Option<Collection>,
    pub unread_only: bool,
    /// Full-text query over downloaded articles.
    pub search: Option<String>,
    pub limit: usize,
    /// Keyset cursor: continue strictly after (timestamp_usec, id) in newest-first order.
    pub after: Option<(i64, ArticleId)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArticlePage {
    pub items: Vec<ArticleSummary>,
    pub next: Option<(i64, ArticleId)>,
}

/// Position inside an article's published content version.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReadingPosition {
    pub content_hash: String,
    /// Text byte offset from the HTML layout engine.
    pub offset: u64,
    /// Text-anchor excerpt used to relocate after a content version change.
    pub anchor: String,
    pub font_size: Option<f32>,
}

/// An explicit, immutable set of article ids a bulk action applies to.
#[derive(Clone, Debug, PartialEq)]
pub struct BulkSelection {
    pub id: i64,
    pub articles: Vec<ArticleId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BulkAction {
    MarkRead,
    MarkUnread,
    Star,
    Unstar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UndoId(pub i64);

/// Result of persisting a local change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommitReceipt {
    pub generation: i64,
    /// False when the desired state equalled the known server state and nothing is pending.
    pub pending: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_forms_round_trip() {
        let id = ArticleId(0xffff_ffff_ffff_fffe);
        assert_eq!(id.to_short(), "-2");
        assert_eq!(id.to_long(), "tag:google.com,2005:reader/item/fffffffffffffffe");
        assert_eq!(ArticleId::parse("-2").unwrap(), id);
        assert_eq!(ArticleId::parse(&id.to_long()).unwrap(), id);
        assert_eq!(ArticleId::from_short("3614359203").unwrap().to_long(), "tag:google.com,2005:reader/item/00000000d7eb6023");
        assert!(ArticleId::from_long("tag:google.com,2005:reader/item/abc").is_err());
        assert_eq!(ArticleId::from_db(id.to_db()), id);
    }

    #[test]
    fn user_ids_are_normalized() {
        assert_eq!(StreamId::new("user/1005921515/label/MTB").0, "user/-/label/MTB");
        assert_eq!(Membership::from_tag_id("user/1005921515/state/com.google/starred"), Some(Membership::Starred));
        assert_eq!(Membership::from_key("label:a b").unwrap().tag_id(), "user/-/label/a b");
    }
}
