//! Typed views of Inoreader responses. The original JSON is always kept alongside.

use crate::error::{Error, Result};
use crate::model::*;
use crate::store::{ArticleRecord, Subscription, TagInfo};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub struct UserInfo {
    pub user_id: String,
    pub user_name: String,
    pub email: String,
}

pub fn parse_user_info(v: &Value) -> Result<UserInfo> {
    Ok(UserInfo {
        user_id: v["userId"].as_str().ok_or_else(|| Error::Data("user-info has no userId".into()))?.to_string(),
        user_name: v["userName"].as_str().unwrap_or("").to_string(),
        email: v["userEmail"].as_str().unwrap_or("").to_string(),
    })
}

fn num_i64(v: &Value) -> i64 {
    match v {
        Value::Number(n) => n.as_i64().unwrap_or(0),
        Value::String(s) => s.parse().unwrap_or(0),
        _ => 0,
    }
}

pub fn parse_subscriptions(v: &Value) -> Result<Vec<Subscription>> {
    let list = v["subscriptions"].as_array().ok_or_else(|| Error::Data("subscription list missing".into()))?;
    let mut out = Vec::new();
    for s in list {
        let id = s["id"].as_str().ok_or_else(|| Error::Data("subscription has no id".into()))?;
        let mut folders = Vec::new();
        if let Some(cats) = s["categories"].as_array() {
            for c in cats {
                if let Some(cid) = c["id"].as_str() {
                    if let Some(name) = normalize_user(cid).strip_prefix("user/-/label/") {
                        folders.push(name.to_string());
                    }
                }
            }
        }
        out.push(Subscription {
            id: id.to_string(),
            title: s["title"].as_str().unwrap_or("").to_string(),
            url: s["url"].as_str().unwrap_or("").to_string(),
            html_url: s["htmlUrl"].as_str().unwrap_or("").to_string(),
            sort_id: s["sortid"].as_str().unwrap_or("").to_string(),
            // Documented as microseconds despite the name.
            first_item_usec: num_i64(&s["firstitemmsec"]),
            folders,
        });
    }
    Ok(out)
}

pub fn parse_tags(v: &Value) -> Result<Vec<TagInfo>> {
    let list = v["tags"].as_array().ok_or_else(|| Error::Data("tag list missing".into()))?;
    Ok(list
        .iter()
        .filter_map(|t| {
            let id = normalize_user(t["id"].as_str()?);
            Some(TagInfo {
                id,
                kind: t["type"].as_str().unwrap_or("tag").to_string(),
                sort_id: t["sortid"].as_str().unwrap_or("").to_string(),
            })
        })
        .collect())
}

#[derive(Clone, Debug, PartialEq)]
pub struct UnreadCounts {
    /// (stream id, count, newest item timestamp usec)
    pub counts: Vec<(String, i64, i64)>,
    /// The per-user cap; a count equal to it means "this many or more".
    pub max: Option<i64>,
}

impl UnreadCounts {
    /// Text for a count, with the cap shown as `N+`.
    pub fn display(&self, count: i64) -> String {
        match self.max {
            Some(m) if count >= m => format!("{m}+"),
            _ => count.to_string(),
        }
    }
}

pub fn parse_unread_counts(v: &Value) -> Result<UnreadCounts> {
    let list = v["unreadcounts"].as_array().ok_or_else(|| Error::Data("unread counts missing".into()))?;
    let counts = list
        .iter()
        .filter_map(|c| Some((normalize_user(c["id"].as_str()?), num_i64(&c["count"]), num_i64(&c["newestItemTimestampUsec"]))))
        .collect();
    let max = if v["max"].is_null() { None } else { Some(num_i64(&v["max"])) };
    Ok(UnreadCounts { counts, max })
}

#[derive(Clone, Debug, PartialEq)]
pub struct StreamPage {
    pub items: Vec<ArticleRecord>,
    pub continuation: Option<String>,
}

pub fn parse_item(item: &Value) -> Result<ArticleRecord> {
    let id = ArticleId::parse(item["id"].as_str().ok_or_else(|| Error::Data("item has no id".into()))?)?;
    let categories: Vec<String> =
        item["categories"].as_array().map(|a| a.iter().filter_map(|c| c.as_str().map(String::from)).collect()).unwrap_or_default();
    let url = item["canonical"][0]["href"].as_str().or_else(|| item["alternate"][0]["href"].as_str()).unwrap_or("").to_string();
    let usec = match &item["timestampUsec"] {
        Value::Null => num_i64(&item["crawlTimeMsec"]) * 1000,
        v => num_i64(v),
    };
    Ok(ArticleRecord {
        id: id.0,
        title: item["title"].as_str().unwrap_or("").to_string(),
        author: item["author"].as_str().unwrap_or("").to_string(),
        origin_stream: normalize_user(item["origin"]["streamId"].as_str().unwrap_or("")),
        origin_title: item["origin"]["title"].as_str().unwrap_or("").to_string(),
        canonical_url: url,
        published: num_i64(&item["published"]),
        timestamp_usec: usec,
        summary_html: item["summary"]["content"].as_str().unwrap_or("").to_string(),
        raw_json: item.to_string(),
        categories,
        // The API gives no excerpt flag; never claim one.
        excerpt: false,
    })
}

pub fn parse_stream_page(v: &Value) -> Result<StreamPage> {
    let items = v["items"].as_array().ok_or_else(|| Error::Data("stream has no items".into()))?;
    let mut out = Vec::with_capacity(items.len());
    for i in items {
        out.push(parse_item(i)?);
    }
    Ok(StreamPage { items: out, continuation: v["continuation"].as_str().filter(|s| !s.is_empty()).map(String::from) })
}

#[derive(Clone, Debug, PartialEq)]
pub struct ItemRef {
    pub id: ArticleId,
    pub timestamp_usec: i64,
    pub direct_stream_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IdPage {
    pub refs: Vec<ItemRef>,
    pub continuation: Option<String>,
}

pub fn parse_id_page(v: &Value) -> Result<IdPage> {
    let refs = v["itemRefs"].as_array().ok_or_else(|| Error::Data("itemRefs missing".into()))?;
    let mut out = Vec::new();
    for r in refs {
        let id = ArticleId::parse(r["id"].as_str().ok_or_else(|| Error::Data("itemRef has no id".into()))?)?;
        out.push(ItemRef {
            id,
            timestamp_usec: num_i64(&r["timestampUsec"]),
            direct_stream_ids: r["directStreamIds"]
                .as_array()
                .map(|a| a.iter().filter_map(|s| s.as_str().map(normalize_user)).collect())
                .unwrap_or_default(),
        });
    }
    Ok(IdPage { refs: out, continuation: v["continuation"].as_str().filter(|s| !s.is_empty()).map(String::from) })
}

/// stream id -> subscription-ordering value (concatenated eight-character sort ids).
pub fn parse_stream_prefs(v: &Value) -> Result<BTreeMap<String, String>> {
    let prefs = v["streamprefs"].as_object().ok_or_else(|| Error::Data("streamprefs missing".into()))?;
    let mut out = BTreeMap::new();
    for (stream, list) in prefs {
        if let Some(arr) = list.as_array() {
            for p in arr {
                if p["id"].as_str() == Some("subscription-ordering") {
                    out.insert(normalize_user(stream), p["value"].as_str().unwrap_or("").to_string());
                }
            }
        }
    }
    Ok(out)
}

/// Splits a `subscription-ordering` value into its eight-character sort ids.
pub fn split_sort_ids(value: &str) -> Vec<String> {
    value.as_bytes().chunks(8).filter(|c| c.len() == 8).map(|c| String::from_utf8_lossy(c).into_owned()).collect()
}

pub fn validate_sort_ids(ids: &[String]) -> Result<String> {
    for id in ids {
        if id.len() != 8 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(Error::InvalidRequest(format!("bad sort id {id:?}")));
        }
    }
    Ok(ids.concat())
}

#[derive(Clone, Debug, PartialEq)]
pub struct QuickAdd {
    pub num_results: i64,
    pub stream_id: Option<String>,
}

pub fn parse_quickadd(v: &Value) -> Result<QuickAdd> {
    if let Some(e) = v["error"].as_str() {
        return Err(Error::InvalidRequest(e.to_string()));
    }
    Ok(QuickAdd { num_results: num_i64(&v["numResults"]), stream_id: v["streamId"].as_str().map(|s| s.to_string()) })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubAction {
    Edit,
    Subscribe,
    Unsubscribe,
}

#[derive(Clone, Debug)]
pub struct SubscriptionEdit {
    pub action: SubAction,
    pub stream: StreamId,
    pub title: Option<String>,
    pub add_folder: Option<String>,
    pub remove_folder: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ContentsParams {
    pub count: u32,
    pub oldest_first: bool,
    pub newer_than: Option<i64>,
    pub exclude_read: bool,
    pub only_starred: bool,
    pub only_liked: bool,
    pub continuation: Option<String>,
    pub annotations: bool,
    pub summaries: bool,
    /// Ask only for manually assigned tags in `categories`.
    pub manual_tags_only: bool,
}

#[derive(Clone, Debug)]
pub struct IdsParams {
    pub stream: StreamId,
    pub count: u32,
    pub oldest_first: bool,
    pub newer_than: Option<i64>,
    pub exclude_read: bool,
    pub only_starred: bool,
    pub only_liked: bool,
    pub continuation: Option<String>,
    pub direct_ids: bool,
}

impl IdsParams {
    pub fn new(stream: StreamId) -> IdsParams {
        IdsParams {
            stream,
            count: 1000,
            oldest_first: false,
            newer_than: None,
            exclude_read: false,
            only_starred: false,
            only_liked: false,
            continuation: None,
            direct_ids: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilterType {
    Folder,
    Subscription,
    Public,
}

#[derive(Clone, Debug)]
pub struct ActiveSearchSpec {
    pub term: String,
    pub filter_type: FilterType,
    pub filter_id: Option<String>,
    pub article_ids: Vec<ArticleId>,
    /// `all`, `titles` or `contents`.
    pub search_filters: Option<String>,
    /// `all_words`, `any_word`, `full_phrase` or `advanced`.
    pub search_match: Option<String>,
    /// `all_sites`, `popular_sites` or `most_popular_sites` (public searches).
    pub search_feed_popularity: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActiveSearchCreated {
    pub id: String,
    pub title: String,
}
