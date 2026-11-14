//! Per-view queries: counts, id lists, folder counts and Inoreader's folder order.

use inoink::content::format_time_left;
use inoink::model::*;
use inoink::store::*;

const STARRED: &str = "user/1/state/com.google/starred";
const READ: &str = "user/1/state/com.google/read";

fn rec(id: u64, ts: i64, feed: &str, cats: &[&str]) -> ArticleRecord {
    ArticleRecord {
        id,
        title: format!("Article {id}"),
        origin_stream: format!("feed/{feed}"),
        origin_title: feed.into(),
        timestamp_usec: ts * 1_000_000,
        published: ts,
        summary_html: format!("<p>body {id}</p>"),
        raw_json: "{}".into(),
        categories: cats.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

fn store() -> Store {
    let mut s = Store::open_in_memory("acct").unwrap();
    s.upsert_article(&rec(1, 10, "a", &[STARRED]), 0).unwrap();
    s.upsert_article(&rec(2, 20, "a", &[READ]), 0).unwrap();
    s.upsert_article(&rec(3, 30, "b", &[]), 0).unwrap();
    s.upsert_article(&rec(4, 40, "b", &[STARRED, READ]), 0).unwrap();
    s
}

fn query(c: Option<Collection>, unread_only: bool) -> ArticleQuery {
    ArticleQuery { collection: c, unread_only, limit: 2, ..Default::default() }
}

fn all_pages(s: &Store, q: &ArticleQuery) -> Vec<u64> {
    let mut q = q.clone();
    let mut out = Vec::new();
    loop {
        let p = s.query_articles(&q).unwrap();
        out.extend(p.items.iter().map(|i| i.id.0));
        match p.next {
            Some(c) => q.after = Some(c),
            None => break,
        }
    }
    out
}

#[test]
fn count_matches_query() {
    let s = store();
    for (c, unread) in [(Some(Collection::All), false), (Some(Collection::All), true), (Some(Collection::Starred), false), (None, true)] {
        let q = query(c, unread);
        assert_eq!(s.count_articles(&q).unwrap(), all_pages(&s, &q).len(), "{:?}", q.collection);
    }
    assert_eq!(s.count_articles(&query(Some(Collection::All), true)).unwrap(), 2);
    assert_eq!(s.count_articles(&query(Some(Collection::Starred), false)).unwrap(), 2);
    let mut q = query(None, false);
    q.search = Some("   ".into());
    assert_eq!(s.count_articles(&q).unwrap(), 0, "a search with nothing to look for matches nothing");
}

#[test]
fn query_ids_follows_order_and_cap() {
    let s = store();
    let q = query(Some(Collection::All), false);
    assert_eq!(s.query_ids(&q, 1000).unwrap().iter().map(|i| i.0).collect::<Vec<_>>(), vec![4, 3, 2, 1]);
    assert_eq!(s.query_ids(&q, 3).unwrap().len(), 3);
    assert_eq!(s.query_ids(&query(Some(Collection::All), true), 1000).unwrap().iter().map(|i| i.0).collect::<Vec<_>>(), vec![3, 1]);
}

#[test]
fn unread_by_folder_counts_only_unread() {
    let mut s = store();
    let sub = |feed: &str, folder: &str| Subscription {
        id: format!("feed/{feed}"),
        title: feed.into(),
        url: feed.into(),
        html_url: String::new(),
        sort_id: String::new(),
        first_item_usec: 0,
        folders: vec![folder.into()],
    };
    s.replace_subscriptions(&[sub("a", "News"), sub("b", "Tech")]).unwrap();
    let counts = s.unread_by_folder().unwrap();
    assert_eq!(counts.get("News"), Some(&1), "article 1 is the only unread one in a");
    assert_eq!(counts.get("Tech"), Some(&1), "article 3 is the only unread one in b");
    s.apply_intent(ArticleId(3), &Membership::Read, true).unwrap();
    assert_eq!(s.unread_by_folder().unwrap().get("Tech"), None);
}

#[test]
fn folders_follow_root_ordering_then_alphabetical() {
    let mut s = store();
    let tag = |n: &str, sid: &str| TagInfo { id: format!("user/-/label/{n}"), kind: "folder".into(), sort_id: sid.into() };
    s.replace_tags(&[tag("Alpha", "00000001"), tag("Beta", "00000002"), tag("Gamma", "00000003"), tag("delta", "00000004")]).unwrap();
    s.set_stream_order(STATE_ROOT, "0000000300000001").unwrap();
    assert_eq!(s.folders_in_order().unwrap(), vec!["Gamma", "Alpha", "Beta", "delta"]);
}

#[test]
fn view_defaults_per_collection() {
    assert!(ViewOptions::default_for(&Collection::All).unread_only);
    assert!(!ViewOptions::default_for(&Collection::Starred).unread_only);
    assert_eq!(ViewOptions::default_for(&Collection::Folder("x".into())).group, ArticleGroup::Unread);
}

#[test]
fn words_come_from_the_content_and_default_to_zero() {
    let s = store();
    assert_eq!(s.words(ArticleId(1)).unwrap(), 2, "\"body 1\"");
    assert_eq!(s.words(ArticleId(999)).unwrap(), 0);
}

#[test]
fn format_time_left_cases() {
    assert_eq!(format_time_left(0), "Less than a minute left");
    assert_eq!(format_time_left(229), "Less than a minute left");
    assert_eq!(format_time_left(230 * 4 + 5), "4 min left");
    assert_eq!(format_time_left(230 * 65), "1 h 5 min left");
    assert_eq!(format_time_left(230 * 120), "2 h left");
}

#[test]
fn unread_unsaved_ids_cover_all_up_to_and_from() {
    let mut s = store();
    s.upsert_article(&rec(5, 50, "b", &[]), 0).unwrap();
    // Newest first: 5, 4 (saved, read), 3, 2 (read), 1 (saved).
    let q = ArticleQuery { collection: Some(Collection::All), unread_only: false, limit: 2, ..Default::default() };
    let ids = |span, max| s.unread_unsaved_ids(&q, span, max).unwrap().iter().map(|i| i.0).collect::<Vec<_>>();
    assert_eq!(ids(ListSpan::All, 1000), vec![5, 3]);
    assert_eq!(ids(ListSpan::UpTo(ArticleId(3)), 1000), vec![5, 3]);
    assert_eq!(ids(ListSpan::UpTo(ArticleId(4)), 1000), vec![5]);
    assert_eq!(ids(ListSpan::From(ArticleId(4)), 1000), vec![3]);
    assert_eq!(ids(ListSpan::From(ArticleId(3)), 1000), vec![3]);
    assert_eq!(ids(ListSpan::UpTo(ArticleId(99)), 1000), Vec::<u64>::new());
    assert_eq!(ids(ListSpan::From(ArticleId(99)), 1000), Vec::<u64>::new());
    assert_eq!(ids(ListSpan::All, 1), vec![5]);
}
