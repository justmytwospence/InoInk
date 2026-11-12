use inoink::error::Error;
use inoink::model::*;
use inoink::store::*;
use tempfile::tempdir;

fn rec(id: u64, ts: i64, cats: &[&str]) -> ArticleRecord {
    ArticleRecord {
        id,
        title: format!("Article {id}"),
        origin_stream: "feed/https://example.org/rss".into(),
        origin_title: "Example".into(),
        timestamp_usec: ts,
        summary_html: format!("<p>body {id}</p>"),
        raw_json: "{}".into(),
        categories: cats.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

fn store() -> Store {
    Store::open_in_memory("acct").unwrap()
}

const STARRED: &str = "user/1/state/com.google/starred";
const READ: &str = "user/1/state/com.google/read";

fn id(n: u64) -> ArticleId {
    ArticleId(n)
}

#[test]
fn intent_is_durable_before_success_and_survives_restart() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    {
        let mut s = Store::open(&path, "acct").unwrap();
        s.upsert_article(&rec(1, 10, &[STARRED]), 0).unwrap();
        let r = s.apply_intent(id(1), &Membership::Starred, false).unwrap();
        assert!(r.pending);
        // The commit is visible to a second connection immediately (durable, not cached).
        let other = Store::open(&path, "acct").unwrap();
        assert!(!other.effective(id(1), &Membership::Starred).unwrap());
        assert_eq!(other.remote(id(1), &Membership::Starred).unwrap(), Some(true));
    }
    let s = Store::open(&path, "acct").unwrap();
    assert_eq!(s.pending_for(id(1)).unwrap(), vec![(Membership::Starred, false)]);
}

#[test]
fn done_marks_read_and_unstars_but_mark_read_preserves_star() {
    let mut s = store();
    s.upsert_article(&rec(1, 10, &[STARRED]), 0).unwrap();
    s.upsert_article(&rec(2, 20, &[STARRED]), 0).unwrap();
    s.done(id(1)).unwrap();
    assert!(s.effective(id(1), &Membership::Read).unwrap());
    assert!(!s.effective(id(1), &Membership::Starred).unwrap());
    s.apply_intent(id(2), &Membership::Read, true).unwrap();
    assert!(s.effective(id(2), &Membership::Read).unwrap());
    assert!(s.effective(id(2), &Membership::Starred).unwrap());
}

#[test]
fn undo_before_and_after_acknowledgement() {
    let mut s = store();
    s.upsert_article(&rec(1, 10, &[STARRED]), 0).unwrap();
    // Before acknowledgement: collapses back to the server state, nothing left to send.
    let u = s.done(id(1)).unwrap();
    s.undo(u).unwrap();
    assert_eq!(s.pending_count().unwrap(), 0);
    assert!(s.effective(id(1), &Membership::Starred).unwrap());
    assert!(!s.effective(id(1), &Membership::Read).unwrap());
    assert!(s.undo(u).is_err(), "undo is single use");

    // After acknowledgement: undo is a durable compensating change that must be sent.
    let u = s.done(id(1)).unwrap();
    for b in s.begin_send(100).unwrap() {
        s.acknowledge(&b).unwrap();
    }
    assert_eq!(s.pending_count().unwrap(), 0);
    s.undo(u).unwrap();
    assert!(s.effective(id(1), &Membership::Starred).unwrap());
    assert!(!s.effective(id(1), &Membership::Read).unwrap());
    assert_eq!(s.pending_count().unwrap(), 2);
}

#[test]
fn repeated_toggles_collapse_to_final_intent() {
    let mut s = store();
    s.upsert_article(&rec(1, 10, &[]), 0).unwrap();
    for i in 0..9 {
        s.apply_intent(id(1), &Membership::Starred, i % 2 == 0).unwrap();
    }
    // Nine toggles, last value true (i=8): one pending intent.
    assert_eq!(s.pending_for(id(1)).unwrap(), vec![(Membership::Starred, true)]);
    s.apply_intent(id(1), &Membership::Starred, false).unwrap();
    assert_eq!(s.pending_count().unwrap(), 0, "back to server state: nothing to send");
}

#[test]
fn change_during_flight_is_not_lost_by_acknowledgement() {
    let mut s = store();
    s.upsert_article(&rec(1, 10, &[]), 0).unwrap();
    s.apply_intent(id(1), &Membership::Starred, true).unwrap();
    let batches = s.begin_send(100).unwrap();
    assert_eq!(batches.len(), 1);
    // While the request is in flight the user flips it back.
    s.apply_intent(id(1), &Membership::Starred, false).unwrap();
    s.acknowledge(&batches[0]).unwrap();
    // Server now has starred=true; the newer local value must remain pending.
    assert_eq!(s.remote(id(1), &Membership::Starred).unwrap(), Some(true));
    assert_eq!(s.pending_for(id(1)).unwrap(), vec![(Membership::Starred, false)]);
    assert!(!s.effective(id(1), &Membership::Starred).unwrap());
    let again = s.begin_send(100).unwrap();
    assert_eq!(again.len(), 1);
    assert!(!again[0].entries[0].2);
}

#[test]
fn lost_response_is_safe_to_retry() {
    let mut s = store();
    s.upsert_article(&rec(1, 10, &[]), 0).unwrap();
    s.apply_intent(id(1), &Membership::Read, true).unwrap();
    let b = s.begin_send(100).unwrap();
    // Request reached the server but the response was lost: we release and resend.
    s.release(&b[0]).unwrap();
    let b2 = s.begin_send(100).unwrap();
    assert_eq!(b2, b, "same state setter is resent unchanged");
    s.acknowledge(&b2[0]).unwrap();
    assert_eq!(s.pending_count().unwrap(), 0);
}

#[test]
fn in_flight_recovered_after_crash() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    {
        let mut s = Store::open(&path, "acct").unwrap();
        s.upsert_article(&rec(1, 10, &[]), 0).unwrap();
        s.apply_intent(id(1), &Membership::Read, true).unwrap();
        assert_eq!(s.begin_send(10).unwrap().len(), 1);
        assert!(s.begin_send(10).unwrap().is_empty(), "already in flight");
    }
    let mut s = Store::open(&path, "acct").unwrap();
    assert_eq!(s.recover_in_flight().unwrap(), 1);
    assert_eq!(s.begin_send(10).unwrap().len(), 1);
}

#[test]
fn done_is_one_request_and_tags_are_independent() {
    let mut s = store();
    s.upsert_article(&rec(1, 10, &[STARRED]), 0).unwrap();
    s.upsert_article(&rec(2, 20, &[STARRED, "user/1/label/keep"]), 0).unwrap();
    s.done(id(1)).unwrap();
    s.done(id(2)).unwrap();
    let batches = s.begin_send(100).unwrap();
    assert_eq!(batches.len(), 1, "both Dones share one add-read/remove-star request");
    assert_eq!(batches[0].add, Some(Membership::Read));
    assert_eq!(batches[0].remove, Some(Membership::Starred));
    assert_eq!(batches[0].articles(), vec![id(1), id(2)]);

    // Adding one tag leaves unrelated memberships alone.
    s.apply_intent(id(2), &Membership::Label("new".into()), true).unwrap();
    assert!(s.effective(id(2), &Membership::Label("keep".into())).unwrap());
    let b = s.begin_send(100).unwrap();
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].add, Some(Membership::Label("new".into())));
    assert_eq!(b[0].remove, None);
}

#[test]
fn batches_are_bounded() {
    let mut s = store();
    for n in 1..=5 {
        s.upsert_article(&rec(n, n as i64, &[]), 0).unwrap();
        s.apply_intent(id(n), &Membership::Starred, true).unwrap();
    }
    let b = s.begin_send(2).unwrap();
    assert_eq!(b.iter().map(|b| b.articles().len()).collect::<Vec<_>>(), vec![2, 2, 1]);
}

#[test]
fn bulk_selection_is_frozen_and_ignores_new_arrivals() {
    let mut s = store();
    for n in 1..=3 {
        s.upsert_article(&rec(n, n as i64, &[]), 0).unwrap();
    }
    let q = ArticleQuery { collection: Some(Collection::Unread), ..Default::default() };
    let sel = s.freeze_selection(&q).unwrap();
    assert_eq!(sel.articles.len(), 3);
    // New articles arrive after the selection was shown.
    s.upsert_article(&rec(4, 4, &[]), 0).unwrap();
    s.upsert_article(&rec(5, 5, &[]), 0).unwrap();
    assert_eq!(s.apply_bulk(&sel, BulkAction::MarkRead).unwrap(), 3);
    assert!(s.effective(id(3), &Membership::Read).unwrap());
    assert!(!s.effective(id(4), &Membership::Read).unwrap());
    assert!(!s.effective(id(5), &Membership::Read).unwrap());
    assert!(s.freeze_ids(&vec![ArticleId(1); BULK_LIMIT + 1]).is_err());
}

#[test]
fn stale_snapshot_cannot_clobber_acknowledged_write() {
    let mut s = store();
    s.upsert_article(&rec(1, 10, &[STARRED]), 0).unwrap();
    let start = s.seq().unwrap();
    // A listing request starts, then a local unstar is acknowledged before its response lands.
    s.apply_intent(id(1), &Membership::Starred, false).unwrap();
    for b in s.begin_send(10).unwrap() {
        s.acknowledge(&b).unwrap();
    }
    // The stale response still says starred.
    s.upsert_article(&rec(1, 10, &[STARRED]), start).unwrap();
    assert_eq!(s.remote(id(1), &Membership::Starred).unwrap(), Some(false));
    // A request started after the acknowledgement is authoritative again.
    let later = s.seq().unwrap();
    s.upsert_article(&rec(1, 10, &[STARRED]), later).unwrap();
    assert_eq!(s.remote(id(1), &Membership::Starred).unwrap(), Some(true));
}

#[test]
fn pending_intent_wins_until_acknowledged_then_remote_wins() {
    let mut s = store();
    s.upsert_article(&rec(1, 10, &[STARRED]), 0).unwrap();
    s.apply_intent(id(1), &Membership::Starred, false).unwrap();
    // Server still reports starred: pending local intent wins in the UI.
    let seq = s.seq().unwrap();
    s.upsert_article(&rec(1, 10, &[STARRED]), seq).unwrap();
    assert!(!s.effective(id(1), &Membership::Starred).unwrap());
    for b in s.begin_send(10).unwrap() {
        s.acknowledge(&b).unwrap();
    }
    // A later observation that the star is back (set elsewhere) wins.
    let seq = s.seq().unwrap();
    s.upsert_article(&rec(1, 10, &[STARRED]), seq).unwrap();
    assert!(s.effective(id(1), &Membership::Starred).unwrap());
}

#[test]
fn reconciliation_never_infers_from_incomplete_listing() {
    let mut s = store();
    for n in 1..=3 {
        s.upsert_article(&rec(n, n as i64, &[STARRED]), 0).unwrap();
    }
    let run = s.begin_reconcile(&Membership::Starred, true).unwrap();
    s.reconcile_page(run, &[id(3)], Some("c1")).unwrap();
    // The listing fails halfway.
    assert!(s.finish_reconciliation(run, false).unwrap().is_empty());
    for n in 1..=3 {
        assert_eq!(s.remote(id(n), &Membership::Starred).unwrap(), Some(true));
    }
    // A complete run proposes candidates; only confirmed ones change, and nothing is deleted.
    let run = s.begin_reconcile(&Membership::Starred, true).unwrap();
    s.reconcile_page(run, &[id(3)], Some("c1")).unwrap();
    let cands = s.finish_reconciliation(run, true).unwrap();
    assert_eq!(cands, vec![id(1), id(2)]);
    assert_eq!(s.resolve_candidates(run, &[id(1)]).unwrap(), 1);
    assert_eq!(s.remote(id(1), &Membership::Starred).unwrap(), Some(false));
    assert_eq!(s.remote(id(2), &Membership::Starred).unwrap(), Some(true));
    assert!(s.has_article(id(1)).unwrap(), "unstarred article is not deleted");
}

#[test]
fn reconciliation_reports_unknown_ids_and_rejects_loops() {
    let mut s = store();
    s.upsert_article(&rec(1, 1, &[]), 0).unwrap();
    let run = s.begin_reconcile(&Membership::Starred, true).unwrap();
    let unknown = s.reconcile_page(run, &[id(1), id(99)], Some("a")).unwrap();
    assert_eq!(unknown, vec![id(99)]);
    assert_eq!(s.remote(id(1), &Membership::Starred).unwrap(), Some(true));
    assert!(matches!(s.reconcile_page(run, &[id(1)], Some("a")), Err(Error::Data(_))));
    assert!(s.reconcile_page(run, &[id(1)], Some("b")).is_err(), "looped run is abandoned");
}

#[test]
fn acknowledgement_during_reconcile_protects_the_write() {
    let mut s = store();
    s.upsert_article(&rec(1, 1, &[]), 0).unwrap();
    let run = s.begin_reconcile(&Membership::Starred, true).unwrap();
    // Local star acknowledged while the listing is being paged.
    s.apply_intent(id(1), &Membership::Starred, true).unwrap();
    for b in s.begin_send(10).unwrap() {
        s.acknowledge(&b).unwrap();
    }
    s.reconcile_page(run, &[], None).unwrap();
    let cands = s.finish_reconciliation(run, true).unwrap();
    assert!(cands.is_empty(), "fresh acknowledgement is not a removal candidate");
}

#[test]
fn queries_by_collection_and_search() {
    let mut s = store();
    s.upsert_article(&rec(1, 10, &[STARRED]), 0).unwrap();
    s.upsert_article(&rec(2, 20, &[READ]), 0).unwrap();
    s.upsert_article(&rec(3, 30, &["user/1/label/tech"]), 0).unwrap();
    fn ids(s: &Store, c: Collection, unread_only: bool) -> Vec<u64> {
        s.query_articles(&ArticleQuery { collection: Some(c), unread_only, ..Default::default() })
            .unwrap()
            .items
            .iter()
            .map(|i| i.id.0)
            .collect::<Vec<_>>()
    }
    assert_eq!(ids(&s, Collection::Starred, false), vec![1]);
    assert_eq!(ids(&s, Collection::Unread, false), vec![3, 1]);
    assert_eq!(ids(&s, Collection::All, true), vec![3, 1]);
    assert_eq!(ids(&s, Collection::All, false), vec![3, 2, 1]);
    assert_eq!(ids(&s, Collection::Tag("tech".into()), false), vec![3]);
    // Unstarring locally removes it from Starred but not from All.
    s.apply_intent(id(1), &Membership::Starred, false).unwrap();
    assert!(ids(&s, Collection::Starred, false).is_empty());
    assert_eq!(ids(&s, Collection::All, false), vec![3, 2, 1]);
    // Keyset pagination.
    let p = s.query_articles(&ArticleQuery { limit: 2, ..Default::default() }).unwrap();
    assert_eq!(p.items.len(), 2);
    let p2 = s.query_articles(&ArticleQuery { limit: 2, after: p.next, ..Default::default() }).unwrap();
    assert_eq!(p2.items.iter().map(|i| i.id.0).collect::<Vec<_>>(), vec![1]);
    assert!(p2.next.is_none());
}

#[test]
fn full_text_search_is_unicode_safe_and_injection_proof() {
    let mut s = store();
    s.upsert_article(&rec(1, 10, &[]), 0).unwrap();
    s.upsert_article(&rec(2, 20, &[]), 0).unwrap();
    s.publish_version(id(1), "r", "h", "d", 1, "Café résumé", "Über naïve cafe", DownloadState::Complete).unwrap();
    s.publish_version(id(2), "r", "h", "d", 1, "Other", "plain text", DownloadState::TextReady).unwrap();
    let search = |t: &str| {
        s.query_articles(&ArticleQuery { search: Some(t.into()), ..Default::default() })
            .map(|p| p.items.iter().map(|i| i.id.0).collect::<Vec<_>>())
    };
    assert_eq!(search("cafe").unwrap(), vec![1]);
    assert_eq!(search("uber NAIVE").unwrap(), vec![1]);
    // FTS syntax in the input is inert: these must neither error nor match everything.
    assert!(search("\"unbalanced").unwrap().is_empty());
    assert!(search("plain OR text NEAR(").is_ok());
    assert!(search("title:*").unwrap().is_empty());
    assert!(search("   ").unwrap().is_empty());
    assert!(search("); DROP TABLE articles; --").unwrap().is_empty());
    assert!(s.has_article(id(1)).unwrap());
}

#[test]
fn positions_round_trip() {
    let mut s = store();
    s.upsert_article(&rec(1, 10, &[]), 0).unwrap();
    let p = ReadingPosition { content_hash: "h1".into(), offset: 4242, anchor: "the quick".into(), font_size: Some(10.5) };
    s.save_position(id(1), &p).unwrap();
    assert_eq!(s.position(id(1)).unwrap(), Some(p));
    assert_eq!(s.position(id(2)).unwrap(), None);
}

#[test]
fn accounts_are_isolated() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let mut a = Store::open(&path, "a").unwrap();
    let mut b = Store::open(&path, "b").unwrap();
    a.upsert_article(&rec(1, 10, &[STARRED]), 0).unwrap();
    a.apply_intent(id(1), &Membership::Starred, false).unwrap();
    assert!(!b.has_article(id(1)).unwrap());
    assert_eq!(b.pending_count().unwrap(), 0);
    assert!(b.begin_send(10).unwrap().is_empty());
    b.upsert_article(&rec(1, 10, &[]), 0).unwrap();
    assert!(!b.effective(id(1), &Membership::Starred).unwrap());
    assert_eq!(a.pending_count().unwrap(), 1);
}

#[test]
fn migration_backs_up_and_rejects_newer_schema() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    {
        let mut s = Store::open(&path, "acct").unwrap();
        s.upsert_article(&rec(1, 10, &[]), 0).unwrap();
        assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
    }
    // Pretend the database is from an older build (version 1 had no 0002 tables).
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        for t in [
            "DROP TABLE shares",
            "DROP TABLE discussions",
            "ALTER TABLE articles DROP COLUMN words",
            "DROP TABLE article_fts",
            "DROP TABLE leases",
            "DROP TABLE assets",
            "DROP TABLE content_versions",
            "DROP TABLE org_ops",
            "DROP TABLE stream_order",
            "DROP TABLE unread_counts",
            "DROP TABLE tags",
            "DROP TABLE subscription_folders",
            "DROP TABLE subscriptions",
        ] {
            c.execute_batch(t).unwrap();
        }
        c.pragma_update(None, "user_version", 1).unwrap();
    }
    {
        let s = Store::open(&path, "acct").unwrap();
        assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
        assert!(s.has_article(id(1)).unwrap(), "data survives migration");
    }
    assert!(dir.path().join("db.bak-v1").exists(), "consistent backup taken before migrating");

    // A database from a newer build is refused, untouched.
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.pragma_update(None, "user_version", SCHEMA_VERSION + 1).unwrap();
    }
    assert!(matches!(Store::open(&path, "acct"), Err(Error::Storage(_))));
}

#[test]
fn failed_migration_rolls_back() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        // A conflicting object makes migration 1 fail midway.
        c.execute_batch("CREATE TABLE sync_tokens(x)").unwrap();
    }
    assert!(Store::open(&path, "acct").is_err());
    let c = rusqlite::Connection::open(&path).unwrap();
    let v: i64 = c.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
    assert_eq!(v, 0);
    let n: i64 = c.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name='articles'", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 0, "partial schema was rolled back");
}

#[test]
fn disk_full_leaves_no_partial_change() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let mut s = Store::open(&path, "acct").unwrap();
    s.upsert_article(&rec(1, 10, &[STARRED]), 0).unwrap();
    let pages: i64 = s.conn().pragma_query_value(None, "page_count", |r| r.get(0)).unwrap();
    s.conn().pragma_update(None, "max_page_count", pages).unwrap();
    // Done writes two intents and an undo row; if the database is full none may persist.
    let mut failed = false;
    for n in 2..400 {
        if s.upsert_article(&rec(n, n as i64, &[STARRED]), 0).is_err() {
            failed = true;
            break;
        }
    }
    assert!(failed, "database should have filled");
    let before = s.pending_count().unwrap();
    for n in 1..50 {
        if s.done(id(n)).is_err() {
            assert_eq!(s.pending_count().unwrap() % 2, 0, "a failed Done leaves no half-applied pair");
            break;
        }
    }
    let _ = before;
    s.conn().pragma_update(None, "max_page_count", 1_000_000).unwrap();
    assert!(s.done(id(1)).is_ok(), "recovers once space returns");
}

fn words_of(s: &Store, n: u64) -> u32 {
    let page = s.query_articles(&ArticleQuery::default()).unwrap();
    page.items.iter().find(|i| i.id == id(n)).unwrap().words
}

#[test]
fn words_follow_summary_then_published_text() {
    let mut s = store();
    let mut r = rec(1, 10, &[]);
    r.summary_html = "<p>one two three</p>".into();
    let seq = s.seq().unwrap();
    s.upsert_article(&r, seq).unwrap();
    assert_eq!(words_of(&s, 1), 3);
    let body = vec!["word"; 500].join(" ");
    s.publish_version(id(1), "raw", "hash", "d/1", 10, "T", &body, DownloadState::Complete).unwrap();
    assert_eq!(words_of(&s, 1), 500);
    let seq = s.seq().unwrap();
    s.upsert_article(&r, seq).unwrap();
    assert_eq!(words_of(&s, 1), 500);
}

#[test]
fn backfill_words_recomputes() {
    let mut s = store();
    let mut r = rec(2, 10, &[]);
    r.summary_html = "<p>a b c d</p>".into();
    let seq = s.seq().unwrap();
    s.upsert_article(&r, seq).unwrap();
    s.conn().execute("UPDATE articles SET words=0", []).unwrap();
    assert_eq!(words_of(&s, 2), 0);
    assert_eq!(s.backfill_words().unwrap(), 1);
    assert_eq!(words_of(&s, 2), 4);
}

fn sorted_store() -> Store {
    let mut s = store();
    // (id, published, title, source, words)
    let data: [(u64, i64, &str, &str, usize); 6] = [
        (1, 1000, "delta", "Zeta Blog", 0),
        (2, 2000, "Alpha", "alpha site", 1),
        (3, 3000, "charlie", "Zeta Blog", 5),
        (4, 4000, "Bravo", "alpha site", 20),
        (5, 5000, "echo", "Zeta Blog", 50),
        (6, 6000, "Foxtrot", "alpha site", 50),
    ];
    for (n, published, title, source, words) in data {
        let mut r = rec(n, n as i64 * 10, &[]);
        r.published = published;
        r.title = title.into();
        r.origin_title = source.into();
        r.summary_html = format!("<p>{}</p>", vec!["w"; words].join(" "));
        let seq = s.seq().unwrap();
        s.upsert_article(&r, seq).unwrap();
    }
    s.apply_intent(id(2), &Membership::Read, true).unwrap();
    s.apply_intent(id(5), &Membership::Read, true).unwrap();
    s
}

fn order(s: &Store, group: ArticleGroup, sort: ArticleSort, limit: usize) -> Vec<u64> {
    let mut out = Vec::new();
    let mut after = None;
    loop {
        let q = ArticleQuery { group, sort, limit, after: after.clone(), ..Default::default() };
        let page = s.query_articles(&q).unwrap();
        out.extend(page.items.iter().map(|i| i.id.0));
        match page.next {
            Some(c) => after = Some(c),
            None => break,
        }
    }
    out
}

#[test]
fn sorts_and_groups_order_and_page_consistently() {
    let s = sorted_store();
    for g in ArticleGroup::ALL {
        for so in ArticleSort::ALL {
            let full = order(&s, g, so, 100);
            let paged = order(&s, g, so, 2);
            assert_eq!(full, paged, "paging differs for {g:?} {so:?}");
            let mut sorted = full.clone();
            sorted.sort();
            assert_eq!(sorted, vec![1, 2, 3, 4, 5, 6], "{g:?} {so:?}");
        }
    }
    let n = ArticleGroup::None;
    assert_eq!(order(&s, n, ArticleSort::Newest, 100), vec![6, 5, 4, 3, 2, 1]);
    assert_eq!(order(&s, n, ArticleSort::Oldest, 100), vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(order(&s, n, ArticleSort::Shortest, 100), vec![2, 3, 4, 6, 5, 1]);
    assert_eq!(order(&s, n, ArticleSort::Longest, 100), vec![6, 5, 4, 3, 2, 1]);
    assert_eq!(order(&s, n, ArticleSort::Title, 100), vec![2, 4, 3, 1, 5, 6]);
    assert_eq!(order(&s, n, ArticleSort::Source, 100), vec![6, 4, 2, 5, 3, 1]);
    // Unread first: unread (1, 3, 4, 6) before read (2, 5).
    assert_eq!(order(&s, ArticleGroup::Unread, ArticleSort::Newest, 100), vec![6, 4, 3, 1, 5, 2]);
    // By source, oldest first inside each source (case-insensitive source order).
    assert_eq!(order(&s, ArticleGroup::Source, ArticleSort::Oldest, 100), vec![2, 4, 6, 1, 3, 5]);
}

#[test]
fn published_zero_falls_back_to_arrival_time() {
    let mut s = store();
    let mut a = rec(1, 5_000_000_000, &[]);
    a.published = 0;
    let mut b = rec(2, 1_000_000, &[]);
    b.published = 4000;
    for r in [&a, &b] {
        let seq = s.seq().unwrap();
        s.upsert_article(r, seq).unwrap();
    }
    // Arrival time 5000 s beats published 4000.
    assert_eq!(order(&s, ArticleGroup::None, ArticleSort::Newest, 100), vec![1, 2]);
}
