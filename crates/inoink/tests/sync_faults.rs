mod common;
use common::fake::*;
use common::*;
use image::{ImageFormat, Rgb, RgbImage};
use inoink::api::*;
use inoink::cache::{Cache, CacheConfig};
use inoink::content::images::{Fetched, Fetcher};
use inoink::error::Result;
use inoink::model::*;
use inoink::store::Store;
use inoink::sync::*;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use url::Url;

const NOW: i64 = 1_800_000_000;
const FEED: &str = "feed/https://blog.example/rss";
const OLD_TS: i64 = 1_400_000_000_000_000; // 2014, far outside the unread window

fn usec(days_ago: i64) -> i64 {
    (NOW - days_ago * 86400) * 1_000_000
}

struct Imgs;
impl Fetcher for Imgs {
    fn get(&self, _u: &Url, _m: usize) -> Result<Fetched> {
        let mut out = Vec::new();
        RgbImage::from_pixel(4, 4, Rgb([1, 2, 3])).write_to(&mut Cursor::new(&mut out), ImageFormat::Png).unwrap();
        Ok(Fetched { status: 200, content_type: Some("image/png".into()), location: None, body: out })
    }
}

struct Rig {
    dir: TempDir,
    fake: Arc<Fake>,
    clock: Arc<FixedClock>,
    tokens: Arc<MemTokens>,
    engine: SyncEngine,
}

fn build(dir: &std::path::Path, fake: &Arc<Fake>, clock: &Arc<FixedClock>, tokens: &Arc<MemTokens>, cfg: SyncConfig) -> SyncEngine {
    let mut acfg = AppConfig::new("cid", "secret", "http://localhost/cb");
    acfg.base_url = "https://example.test/reader/api/0".into();
    acfg.token_url = "https://example.test/oauth2/token".into();
    let api = ApiClient::new(acfg, fake.clone(), clock.clone(), tokens.clone()).unwrap();
    let store = Store::open(&dir.join("db.sqlite"), "42").unwrap();
    let cache = Cache::new(&dir.join("cache"), "42", CacheConfig::default());
    let mut e = SyncEngine::new(store, api, cache, Arc::new(Imgs), clock.clone(), cfg).with_sleep(Arc::new(|_| {}));
    e.recover().unwrap();
    e
}

fn rig_with(cfg: SyncConfig) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    fake.with(|s| {
        s.subs.push(SSub {
            id: FEED.into(),
            title: "Blog".into(),
            url: "https://blog.example/rss".into(),
            folders: vec!["Tech".into()],
            first_usec: 0,
        })
    });
    let clock = FixedClock::new(NOW);
    let tokens = Arc::new(MemTokens::default());
    *tokens.tokens.lock().unwrap() = Some(tokens_at(NOW + 3600));
    let engine = build(dir.path(), &fake, &clock, &tokens, cfg);
    Rig { dir, fake, clock, tokens, engine }
}

fn rig() -> Rig {
    rig_with(SyncConfig::default())
}

fn tokens_at(expires: i64) -> Tokens {
    Tokens { access_token: "acc".into(), refresh_token: "ref".into(), expires_at: expires, scope: "read write".into() }
}

impl Rig {
    fn restart(&mut self) {
        let path: PathBuf = self.dir.path().to_path_buf();
        // Dropping the old engine closes its store, as a process exit would.
        let old = std::mem::replace(&mut self.engine, build(&path, &self.fake, &self.clock, &self.tokens, SyncConfig::default()));
        drop(old);
    }
    fn eff(&self, id: u64, m: Membership) -> bool {
        self.engine.store.effective(ArticleId(id), &m).unwrap()
    }
    fn full(&mut self) -> CycleReport {
        self.engine.run_cycle(Reason::Explicit)
    }
}

fn seed(r: &Rig) {
    for i in 1..=3u64 {
        r.fake.art(i, usec(i as i64), FEED);
    }
    r.fake.art(10, OLD_TS, FEED);
    r.fake.with(|s| {
        s.arts.get_mut(&10).unwrap().starred = true;
        s.arts.get_mut(&1).unwrap().starred = true;
    });
}

#[test]
fn first_sync_fetches_unread_and_downloads_old_starred_articles() {
    let mut r = rig();
    seed(&r);
    let rep = r.full();
    assert_eq!(rep.error, None);
    assert_eq!(rep.reconciled, vec!["starred", "unread", "archive", "annotated", "progress"]);
    for id in [1, 2, 3, 10] {
        assert!(r.engine.store.has_article(ArticleId(id)).unwrap(), "{id}");
        assert!(r.engine.store.download_state(ArticleId(id)).unwrap().readable_offline(), "{id} cached");
    }
    assert!(r.eff(10, Membership::Starred) && r.eff(1, Membership::Starred));
    assert!(!r.eff(10, Membership::Read), "old starred article is unread on the server");
    assert!(!r.eff(2, Membership::Starred));
    assert!(!r.eff(2, Membership::Label("Tech".into())), "inherited folder tags are not article tags");
    assert_eq!(r.engine.store.subscriptions().unwrap()[0].folders, vec!["Tech"]);
}

#[test]
fn old_article_starred_elsewhere_arrives_and_is_cached() {
    let mut r = rig();
    seed(&r);
    r.full();
    r.fake.art(99, 1_100_000_000_000_000, FEED); // 2004: never seen locally
    r.fake.with(|s| s.arts.get_mut(&99).unwrap().starred = true);
    assert!(!r.engine.store.has_article(ArticleId(99)).unwrap());
    let rep = r.full();
    assert_eq!(rep.error, None);
    assert!(r.eff(99, Membership::Starred));
    assert!(r.engine.store.download_state(ArticleId(99)).unwrap().readable_offline(), "cached despite its age");
}

#[test]
fn remote_unstar_updates_state_without_deleting_or_leaving_other_collections() {
    let mut r = rig();
    seed(&r);
    r.full();
    r.fake.clear_log();
    r.fake.with(|s| s.arts.get_mut(&10).unwrap().starred = false);
    assert_eq!(r.full().error, None);
    assert!(!r.eff(10, Membership::Starred));
    assert!(r.engine.store.has_article(ArticleId(10)).unwrap());
    let all = r.engine.store.query_articles(&ArticleQuery { collection: Some(Collection::All), ..Default::default() }).unwrap();
    assert!(all.items.iter().any(|i| i.id == ArticleId(10)), "still in All articles");
    let starred = r.engine.store.query_articles(&ArticleQuery { collection: Some(Collection::Starred), ..Default::default() }).unwrap();
    assert_eq!(starred.items.iter().map(|i| i.id.0).collect::<Vec<_>>(), vec![1]);
    // The removal was verified by a second complete listing.
    let listings = r.fake.log().iter().filter(|l| l.contains("items/ids") && l.contains("it=user")).count();
    assert_eq!(listings, 2);
}

#[test]
fn finishing_offline_survives_restart_and_syncs_on_reconnect() {
    let mut r = rig();
    seed(&r);
    r.full();
    r.fake.fail("", Kind::Offline, 1000);
    r.engine.store.archive(ArticleId(1)).unwrap();
    let rep = r.engine.run_cycle(Reason::Local);
    assert_eq!(rep.error, Some(SyncError::Offline));
    assert_eq!(r.engine.store.pending_count().unwrap(), 2);
    assert!(r.eff(1, Membership::Archived) && !r.eff(1, Membership::Starred), "UI shows the Archive immediately");
    r.restart();
    assert_eq!(r.engine.store.pending_count().unwrap(), 2, "survives restart");
    r.fake.with(|s| s.faults.clear());
    r.fake.clear_log();
    let rep = r.engine.run_cycle(Reason::Wake);
    assert_eq!(rep.error, None);
    let a = r.fake.a(1);
    assert!(a.archived && !a.starred && !a.read);
    assert_eq!(r.engine.store.pending_count().unwrap(), 0);
    assert_eq!(r.fake.count("edit-tag"), 1, "Archive is a single request");
    assert!(r
        .fake
        .log()
        .iter()
        .any(|l| l.contains("a=user/-/state/com.google/archived") && l.contains("r=user/-/state/com.google/starred")));
}

#[test]
fn mark_read_keeps_the_star_on_the_server() {
    let mut r = rig();
    seed(&r);
    r.full();
    r.engine.store.apply_intent(ArticleId(1), &Membership::Read, true).unwrap();
    assert_eq!(r.engine.run_cycle(Reason::Local).error, None);
    let a = r.fake.a(1);
    assert!(a.read && a.starred);
}

#[test]
fn lost_response_is_retried_and_converges() {
    let mut r = rig();
    seed(&r);
    r.full();
    r.fake.fail("/edit-tag", Kind::LostResponse, 1);
    r.engine.store.apply_intent(ArticleId(2), &Membership::Starred, true).unwrap();
    let rep = r.engine.run_cycle(Reason::Local);
    assert!(rep.error.is_some(), "the response was lost");
    assert!(r.fake.a(2).starred, "but the server did apply it");
    assert_eq!(r.engine.store.pending_count().unwrap(), 1, "still pending locally");
    r.fake.clear_log();
    assert_eq!(r.engine.run_cycle(Reason::Local).error, None);
    assert_eq!(r.engine.store.pending_count().unwrap(), 0);
    assert!(r.fake.a(2).starred, "resending a state setter is harmless");
    assert_eq!(r.fake.count("edit-tag"), 1);
}

#[test]
fn partial_failure_acknowledges_only_what_succeeded() {
    let mut r = rig();
    seed(&r);
    r.full();
    r.engine.store.apply_intent(ArticleId(2), &Membership::Read, true).unwrap(); // batch A
    r.engine.store.apply_intent(ArticleId(3), &Membership::Starred, true).unwrap(); // batch B
    r.fake.fail_after("/edit-tag", 1, Kind::Status(503), 1);
    let rep = r.engine.run_cycle(Reason::Local);
    assert_eq!(rep.sent_batches, 1);
    assert!(rep.error.is_some());
    let pending: usize = r.engine.store.pending_count().unwrap();
    assert_eq!(pending, 1);
    assert_eq!(r.fake.a(2).read as u8 + r.fake.a(3).starred as u8, 1, "exactly one change reached the server");
    assert_eq!(r.engine.run_cycle(Reason::Local).error, None);
    assert!(r.fake.a(2).read && r.fake.a(3).starred);
}

#[test]
fn requests_in_flight_at_a_crash_are_resent_after_restart() {
    let mut r = rig();
    seed(&r);
    r.full();
    r.engine.store.apply_intent(ArticleId(2), &Membership::Read, true).unwrap();
    let batches = r.engine.store.begin_send(10).unwrap();
    assert_eq!(batches.len(), 1);
    r.restart(); // process died after marking in flight
    assert_eq!(r.engine.run_cycle(Reason::Local).error, None);
    assert!(r.fake.a(2).read);
    assert_eq!(r.engine.store.pending_count().unwrap(), 0);
}

#[test]
fn repeated_toggles_send_only_the_final_state() {
    let mut r = rig();
    seed(&r);
    r.full();
    r.fake.clear_log();
    for i in 0..7 {
        r.engine.store.apply_intent(ArticleId(2), &Membership::Starred, i % 2 == 0).unwrap();
        r.engine.store.apply_intent(ArticleId(3), &Membership::Read, true).unwrap();
        r.engine.store.apply_intent(ArticleId(3), &Membership::Read, false).unwrap();
    }
    r.engine.run_cycle(Reason::Local);
    assert!(r.fake.a(2).starred);
    assert!(!r.fake.a(3).read);
    assert_eq!(r.fake.count("edit-tag"), 1, "article 3 was back to its server state: nothing to send for it");
}

#[test]
fn tag_changes_do_not_overwrite_other_memberships() {
    let mut r = rig();
    seed(&r);
    r.fake.with(|s| {
        s.arts.get_mut(&2).unwrap().labels.insert("keep".into());
    });
    r.full();
    r.engine.store.apply_intent(ArticleId(2), &Membership::Label("new".into()), true).unwrap();
    r.engine.run_cycle(Reason::Local);
    let a = r.fake.a(2);
    assert!(a.labels.contains("keep") && a.labels.contains("new"));
    r.engine.store.apply_intent(ArticleId(2), &Membership::Label("keep".into()), false).unwrap();
    r.engine.run_cycle(Reason::Local);
    let a = r.fake.a(2);
    assert!(!a.labels.contains("keep") && a.labels.contains("new"));
}

#[test]
fn pending_intent_wins_until_acknowledged_then_remote_changes_win() {
    let mut r = rig();
    seed(&r);
    r.full();
    // Offline: local unstar of #1 while the server still has the star.
    r.fake.fail("/edit-tag", Kind::Offline, 1);
    r.engine.store.apply_intent(ArticleId(1), &Membership::Starred, false).unwrap();
    r.full();
    assert!(!r.eff(1, Membership::Starred), "pending local intent wins over the server's older value");
    assert_eq!(r.engine.store.remote(ArticleId(1), &Membership::Starred).unwrap(), Some(true));
    // Back online it is sent and acknowledged.
    assert_eq!(r.engine.run_cycle(Reason::Local).error, None);
    assert!(!r.fake.a(1).starred);
    // Someone re-stars it elsewhere: the later observation wins.
    r.fake.with(|s| s.arts.get_mut(&1).unwrap().starred = true);
    r.full();
    assert!(r.eff(1, Membership::Starred));
}

#[test]
fn partial_failed_looped_or_capped_listings_never_remove_anything() {
    // Failure on the second page.
    let mut r = rig();
    seed(&r);
    r.fake.with(|s| s.page_cap = 1);
    r.full();
    r.fake.with(|s| {
        s.arts.get_mut(&10).unwrap().starred = false;
        s.arts.get_mut(&1).unwrap().starred = false;
    });
    r.fake.fail_after("/stream/items/ids", 1, Kind::Status(503), 1);
    let rep = r.full();
    assert!(rep.error.is_some());
    assert!(r.eff(10, Membership::Starred) && r.eff(1, Membership::Starred), "a failed listing is not evidence of an unstar");

    // A looping continuation token abandons the run.
    let mut r = rig();
    seed(&r);
    r.full();
    r.fake.with(|s| {
        s.arts.get_mut(&10).unwrap().starred = false;
        s.page_cap = 1;
    });
    r.fake.fail("/stream/items/ids", Kind::LoopToken, 100);
    let rep = r.full();
    assert!(rep.error.is_some());
    assert!(r.eff(10, Membership::Starred));

    // Hitting the page cap marks the listing incomplete and removes nothing.
    let mut r = rig();
    seed(&r);
    r.fake.with(|s| s.arts.get_mut(&2).unwrap().starred = true);
    r.full();
    let path = r.dir.path().to_path_buf();
    let capped = build(&path, &r.fake, &r.clock, &r.tokens, SyncConfig { id_page_cap: 1, ..Default::default() });
    drop(std::mem::replace(&mut r.engine, capped));
    r.fake.with(|s| {
        s.page_cap = 1;
        s.arts.get_mut(&10).unwrap().starred = false;
    });
    let rep = r.full();
    assert!(rep.incomplete.contains(&"starred"), "{rep:?}");
    assert!(r.eff(10, Membership::Starred), "incomplete listing: no inferred unstars");
}

#[test]
fn removal_candidates_must_be_confirmed_by_a_second_listing() {
    let mut r = rig();
    seed(&r);
    r.full();
    // The server's first listing wrongly omits #1 (transient inconsistency); the second is right.
    r.fake.fail(
        "/stream/items/ids",
        Kind::Hook(Box::new(|s| {
            s.arts.get_mut(&1).unwrap().starred = false;
        })),
        1,
    );
    r.fake.fail(
        "/stream/items/ids",
        Kind::Hook(Box::new(|s| {
            s.arts.get_mut(&1).unwrap().starred = true;
        })),
        1,
    );
    r.full();
    assert!(r.eff(1, Membership::Starred), "disagreeing passes do not remove a membership");
}

#[test]
fn multi_page_listings_and_contents_paging_work() {
    let mut r = rig();
    for i in 1..=7u64 {
        r.fake.art(i, usec(i as i64), FEED);
    }
    r.fake.with(|s| {
        s.page_cap = 3;
        for i in [2, 4, 6] {
            s.arts.get_mut(&i).unwrap().starred = true;
        }
    });
    let rep = r.full();
    assert_eq!(rep.error, None, "{rep:?}");
    assert_eq!(rep.missing, 0);
    assert_eq!(r.engine.store.query_articles(&ArticleQuery { limit: 100, ..Default::default() }).unwrap().items.len(), 7);
}

#[test]
fn old_article_cannot_be_marked_unread_and_the_change_is_dropped() {
    let mut r = rig();
    seed(&r);
    r.fake.with(|s| s.subs[0].first_usec = usec(2));
    r.full();
    // #3 is 3 days old: older than the feed's first item (2 days ago).
    r.engine.store.apply_intent(ArticleId(3), &Membership::Read, true).unwrap();
    r.engine.run_cycle(Reason::Local);
    r.fake.clear_log();
    r.engine.store.apply_intent(ArticleId(3), &Membership::Read, false).unwrap();
    r.engine.store.apply_intent(ArticleId(3), &Membership::Starred, true).unwrap();
    let rep = r.engine.run_cycle(Reason::Local);
    assert_eq!(rep.cannot_unread, vec![ArticleId(3)]);
    assert!(r.eff(3, Membership::Read), "the UI reverts to the server's state");
    assert!(r.fake.a(3).starred, "its other change still went through");
    assert!(r.fake.a(3).read);
    assert!(r.fake.log().iter().all(|l| !l.contains("r=user/-/state/com.google/read")), "never sent");
}

#[test]
fn silently_ignored_unread_is_corrected_by_the_next_reconciliation() {
    let mut r = rig();
    seed(&r);
    // The server knows the boundary, but our subscription copy does not (first_item unknown locally).
    r.full();
    r.fake.with(|s| {
        s.subs[0].first_usec = usec(0) + 1;
        s.arts.get_mut(&2).unwrap().read = true;
    });
    r.full(); // learns #2 is read
    assert!(r.eff(2, Membership::Read));
    r.engine.store.set_meta("noop", "1").unwrap();
    // Stale local subscription (no boundary): force it by clearing it.
    r.engine.store.replace_subscriptions(&[]).unwrap();
    r.engine.store.apply_intent(ArticleId(2), &Membership::Read, false).unwrap();
    r.engine.run_cycle(Reason::Local);
    assert!(r.fake.a(2).read, "the server said OK but ignored it");
    r.full();
    assert!(r.eff(2, Membership::Read), "reconciliation shows the truth");
}

#[test]
fn token_rotation_expiry_and_revocation() {
    let mut r = rig();
    seed(&r);
    r.full();
    // Expired access token: refreshed once, rotation persisted.
    r.clock.advance(4000);
    r.engine.store.apply_intent(ArticleId(2), &Membership::Read, true).unwrap();
    assert_eq!(r.engine.run_cycle(Reason::Local).error, None);
    let saves = r.tokens.saves.lock().unwrap().clone();
    assert_eq!(saves.len(), 1);
    assert_eq!(saves[0].refresh_token, "ref2");
    // The server revokes the refresh token: sign in again, nothing lost.
    r.fake.with(|s| {
        s.valid_refresh = "revoked".into();
        s.valid_access = "other".into();
    });
    r.engine.store.apply_intent(ArticleId(3), &Membership::Read, true).unwrap();
    let rep = r.engine.run_cycle(Reason::Local);
    assert_eq!(rep.error, Some(SyncError::NeedsSignIn));
    assert_eq!(r.engine.store.pending_count().unwrap(), 1);
    assert!(r.eff(3, Membership::Read));
}

#[test]
fn quota_exhaustion_pauses_visibly_and_resumes_without_losing_work() {
    let mut r = rig();
    seed(&r);
    r.fake.with(|s| {
        s.limits = [100, 100];
        s.usage = [100, 100];
        s.reset_after = 600;
    });
    r.engine.store.apply_intent(ArticleId(2), &Membership::Read, true).unwrap();
    let rep = r.engine.run_cycle(Reason::Local);
    assert_eq!(rep.error, Some(SyncError::QuotaPaused { secs: 600 }));
    assert_eq!(r.engine.store.pending_count().unwrap(), 1);
    // Still paused (even after restart): no request is made.
    r.restart();
    r.fake.clear_log();
    let rep = r.engine.run_cycle(Reason::Local);
    assert!(matches!(rep.error, Some(SyncError::QuotaPaused { .. })));
    assert!(r.fake.log().is_empty(), "no request while paused");
    // After the reset, work resumes.
    r.clock.advance(601);
    r.fake.with(|s| s.usage = [0, 0]);
    assert_eq!(r.engine.run_cycle(Reason::Local).error, None);
    assert!(r.fake.a(2).read);
}

#[test]
fn background_sync_leaves_a_reserve_for_explicit_actions() {
    let mut r = rig();
    seed(&r);
    r.fake.with(|s| {
        s.limits = [100, 100];
        s.usage = [0, 92]; // 8 writes left; reserve is 10
        s.reset_after = 600;
    });
    r.engine.store.apply_intent(ArticleId(2), &Membership::Read, true).unwrap();
    // One request teaches the client the server's numbers.
    let _ = r.engine.api(Priority::Explicit).user_info();
    let rep = r.engine.run_cycle(Reason::Local);
    assert!(matches!(rep.error, Some(SyncError::QuotaPaused { .. })), "background stops at the reserve");
    assert_eq!(r.engine.store.pending_count().unwrap(), 1);
    // The user's explicit action may use it.
    let rep = r.engine.run_cycle(Reason::Explicit);
    assert!(!matches!(rep.error, Some(SyncError::QuotaPaused { .. })) || r.fake.a(2).read);
    assert!(r.fake.a(2).read);
}

#[test]
fn reads_exhausted_does_not_block_writes_in_the_other_zone() {
    let mut r = rig();
    seed(&r);
    r.fake.with(|s| {
        s.limits = [5, 100];
        s.usage = [5, 0];
    });
    r.engine.store.apply_intent(ArticleId(2), &Membership::Read, true).unwrap();
    let rep = r.engine.run_cycle(Reason::Explicit);
    assert!(r.fake.a(2).read, "zone 2 write went through");
    assert!(matches!(rep.error, Some(SyncError::QuotaPaused { .. })), "zone 1 refresh was refused: {rep:?}");
}

#[test]
fn automatic_cycles_respect_the_minimum_interval_but_flush_writes() {
    let mut r = rig();
    seed(&r);
    r.full();
    r.fake.clear_log();
    r.engine.store.apply_intent(ArticleId(2), &Membership::Read, true).unwrap();
    let rep = r.engine.run_cycle(Reason::Wake);
    assert!(rep.skipped_interval);
    assert_eq!(r.fake.count("items/ids"), 0);
    assert!(r.fake.a(2).read, "writes are not delayed by the interval");
    r.clock.advance(901);
    let rep = r.engine.run_cycle(Reason::Wake);
    assert!(!rep.skipped_interval);
    assert_eq!(rep.unchanged, vec!["starred", "unread"], "the unread counts show nothing changed elsewhere");
    assert_eq!(r.fake.count("items/ids"), 0);
    assert_eq!(r.fake.count("/unread-count"), 1);
    assert_eq!(r.fake.count("/subscription/list"), 0, "the organization is not due");
    assert_eq!(rep.requests, 1, "a sync with nothing new costs one request");
    // Read elsewhere: the count changes and the unread listing runs again.
    r.fake.with(|s| s.arts.get_mut(&3).unwrap().read = true);
    r.clock.advance(901);
    let rep = r.engine.run_cycle(Reason::Wake);
    assert_eq!(rep.reconciled, vec!["unread"]);
    assert!(r.eff(3, Membership::Read));
    assert_eq!(r.fake.count("items/ids"), 2, "listed, and the absence confirmed");
}

#[test]
fn unchanged_counts_still_list_again_after_the_maximum_age() {
    let mut r = rig();
    seed(&r);
    r.full();
    r.fake.with(|s| s.arts.get_mut(&3).unwrap().read = true);
    r.full();
    // Read one article and mark another unread elsewhere: the count and the newest unread
    // article stay the same.
    r.fake.with(|s| {
        s.arts.get_mut(&2).unwrap().read = true;
        s.arts.get_mut(&3).unwrap().read = false;
    });
    let rep = r.full();
    assert!(rep.unchanged.contains(&"unread"));
    r.clock.advance(SyncConfig::default().reconcile_max_age_secs);
    let rep = r.full();
    assert!(rep.reconciled.contains(&"unread"));
    assert!(r.eff(2, Membership::Read) && !r.eff(3, Membership::Read));
}

#[test]
fn a_feed_followed_elsewhere_refreshes_the_organization() {
    let mut r = rig();
    seed(&r);
    r.full();
    r.fake.clear_log();
    r.full();
    assert_eq!(r.fake.count("/subscription/list"), 0);
    r.fake.with(|s| {
        s.subs.push(SSub {
            id: "feed/https://other.example/rss".into(),
            title: "Other".into(),
            url: "https://other.example/rss".into(),
            folders: vec![],
            first_usec: 0,
        })
    });
    let rep = r.full();
    assert!(rep.refreshed_org);
    assert!(r.engine.store.subscription("feed/https://other.example/rss").unwrap().is_some());
}

#[test]
fn organization_changes_apply_locally_and_remote_changes_arrive() {
    let mut r = rig();
    seed(&r);
    r.full();
    // Remote: the feed moves to another folder and is renamed.
    r.fake.with(|s| {
        s.subs[0].folders = vec!["News".into()];
        s.subs[0].title = "Blog Renamed".into();
    });
    r.full();
    let sub = r.engine.store.subscription(FEED).unwrap().unwrap();
    assert_eq!((sub.title.as_str(), sub.folders.clone()), ("Blog Renamed", vec!["News".to_string()]));

    // Local organization change shows immediately and is not overwritten by a refresh before it is sent.
    r.fake.fail("/subscription/edit", Kind::Offline, 1);
    r.engine
        .store
        .enqueue_org(
            "subscription_edit",
            &serde_json::json!({"action": "edit", "stream": FEED, "title": "Mine", "add_folder": "Tech", "remove_folder": "News"})
                .to_string(),
        )
        .unwrap();
    assert_eq!(r.engine.store.subscription(FEED).unwrap().unwrap().title, "Mine");
    r.full();
    assert_eq!(r.engine.store.subscription(FEED).unwrap().unwrap().title, "Mine", "pending local organization wins");
    assert_eq!(r.engine.run_cycle(Reason::Local).error, None);
    r.full();
    let sub = r.engine.store.subscription(FEED).unwrap().unwrap();
    assert_eq!((sub.title.as_str(), sub.folders.clone()), ("Mine", vec!["Tech".to_string()]));
    assert!(!r.engine.store.org_pending().unwrap());
}

#[test]
fn unsubscribing_keeps_articles_and_adding_a_feed_after_a_lost_response_is_not_duplicated() {
    let mut r = rig();
    seed(&r);
    r.full();
    r.engine.store.enqueue_org("subscription_edit", &serde_json::json!({"action": "unsubscribe", "stream": FEED}).to_string()).unwrap();
    assert!(r.engine.store.subscription(FEED).unwrap().is_none());
    assert!(r.engine.store.has_article(ArticleId(2)).unwrap(), "articles survive unsubscribing");
    r.engine.run_cycle(Reason::Local);
    assert!(r.fake.with(|s| s.subs.is_empty()));

    // quickadd: applied on the server, response lost.
    r.fake.fail("/subscription/quickadd", Kind::LostResponse, 1);
    r.engine.store.enqueue_org("quickadd", &serde_json::json!({"feed": "https://new.example/rss"}).to_string()).unwrap();
    r.engine.run_cycle(Reason::Local);
    assert_eq!(r.engine.store.org_ops().unwrap()[0].state, "needs_reconciliation");
    r.fake.clear_log();
    r.engine.run_cycle(Reason::Local);
    assert!(r.engine.store.org_ops().unwrap().is_empty(), "reconciled against the subscription list");
    assert_eq!(r.fake.count("quickadd"), 0, "not sent twice");
    assert_eq!(r.fake.with(|s| s.subs.len()), 1);
}

#[test]
fn rename_with_lost_response_is_reconciled_not_repeated() {
    let mut r = rig();
    seed(&r);
    r.full();
    r.fake.fail("/rename-tag", Kind::LostResponse, 1);
    r.engine
        .store
        .enqueue_org("rename_tag", &serde_json::json!({"source": "user/-/label/Tech", "dest": "Technology"}).to_string())
        .unwrap();
    r.engine.run_cycle(Reason::Local);
    r.fake.clear_log();
    r.engine.run_cycle(Reason::Local);
    assert_eq!(r.fake.count("rename-tag"), 0);
    assert!(r.engine.store.org_ops().unwrap().is_empty());
    assert_eq!(r.fake.with(|s| s.subs[0].folders.clone()), vec!["Technology".to_string()]);
}

#[test]
fn active_search_with_unknown_outcome_is_never_blindly_repeated() {
    let mut r = rig();
    seed(&r);
    r.full();
    r.fake.fail("/active_search/create", Kind::LostResponse, 1);
    r.engine
        .store
        .enqueue_org(
            "active_search_create",
            &serde_json::json!({"term": "rust", "filter_type": "folder", "filter_id": "user/-/label/Tech"}).to_string(),
        )
        .unwrap();
    r.engine.run_cycle(Reason::Local);
    let op = &r.engine.store.org_ops().unwrap()[0];
    assert_eq!(op.state, "failed");
    r.fake.clear_log();
    r.engine.run_cycle(Reason::Local);
    assert_eq!(r.fake.count("active_search/create"), 0);
}

#[test]
fn rejected_organization_changes_are_reverted_by_the_next_refresh() {
    let mut r = rig();
    seed(&r);
    r.full();
    r.fake.fail("/subscription/edit", Kind::Status(400), 1);
    r.engine
        .store
        .enqueue_org("subscription_edit", &serde_json::json!({"action": "edit", "stream": FEED, "title": "Nope"}).to_string())
        .unwrap();
    r.engine.run_cycle(Reason::Local);
    assert_eq!(r.engine.store.org_ops().unwrap()[0].state, "failed");
    r.full();
    assert_eq!(r.engine.store.subscription(FEED).unwrap().unwrap().title, "Blog", "server's value restored");
}

#[test]
fn storage_pressure_pauses_downloads_without_failing_the_sync() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    fake.with(|s| s.subs.push(SSub { id: FEED.into(), title: "Blog".into(), url: String::new(), folders: vec![], first_usec: 0 }));
    let clock = FixedClock::new(NOW);
    let tokens = Arc::new(MemTokens::default());
    *tokens.tokens.lock().unwrap() = Some(tokens_at(NOW + 3600));
    let mut e = build(dir.path(), &fake, &clock, &tokens, SyncConfig::default()).with_free_space(Arc::new(|| Some(1024)));
    fake.art(1, usec(1), FEED);
    let rep = e.run_cycle(Reason::Explicit);
    assert_eq!(rep.error, None);
    assert!(rep.storage_paused);
    assert!(e.store.has_article(ArticleId(1)).unwrap(), "metadata is still synced");
    assert_eq!(e.store.download_state(ArticleId(1)).unwrap(), DownloadState::NotDownloaded);
}

#[test]
fn browsing_history_pages_with_continuation() {
    let mut r = rig();
    for i in 1..=5u64 {
        r.fake.art(i, usec(i as i64 * 40), FEED);
    }
    r.fake.with(|s| s.page_cap = 2);
    let mut cont = None;
    let mut pages = 0;
    loop {
        cont = r.engine.browse(&Collection::All, cont).unwrap();
        pages += 1;
        if cont.is_none() {
            break;
        }
    }
    assert_eq!(pages, 3);
    assert_eq!(r.engine.store.query_articles(&ArticleQuery { limit: 100, ..Default::default() }).unwrap().items.len(), 5);
}

fn annotated_requests(r: &Rig) -> usize {
    r.fake.log().iter().filter(|l| l.contains("annotated")).count()
}

fn add_annotation(r: &Rig, id: u64, ann_id: i64) {
    r.fake.with(|s| {
        s.arts
            .get_mut(&id)
            .unwrap()
            .annotations
            .push(serde_json::json!({"id": ann_id, "start": 3, "end": 9, "text": "Body", "note": "n", "added_on": 1_700_000_000}))
    });
}

#[test]
fn annotations_refresh_on_explicit_and_interval() {
    let mut r = rig();
    r.fake.art(1, usec(1), FEED);
    add_annotation(&r, 1, 5);
    assert_eq!(r.full().error, None);
    assert_eq!(annotated_requests(&r), 1);
    assert_eq!(r.engine.store.remote_annotations(ArticleId(1)).unwrap().len(), 1);

    // A background cycle soon after reads nothing from the Annotated stream.
    r.fake.clear_log();
    r.clock.advance(1000);
    assert_eq!(r.engine.run_cycle(Reason::Wake).error, None);
    assert_eq!(annotated_requests(&r), 0);

    // Past six hours it does.
    r.clock.advance(21_601);
    *r.tokens.tokens.lock().unwrap() = Some(tokens_at(NOW + 1000 + 21_601 + 3600));
    assert_eq!(r.engine.run_cycle(Reason::Wake).error, None);
    assert_eq!(annotated_requests(&r), 1);

    // An annotation added later on another device arrives on the next explicit sync.
    add_annotation(&r, 1, 6);
    assert_eq!(r.full().error, None);
    assert_eq!(r.engine.store.remote_annotations(ArticleId(1)).unwrap().len(), 2);
}

#[test]
fn annotations_cleared_only_after_complete_listing() {
    let mut r = rig();
    r.fake.art(1, usec(1), FEED);
    add_annotation(&r, 1, 5);
    r.full();
    r.fake.with(|s| s.arts.get_mut(&1).unwrap().annotations.clear());
    let rep = r.full();
    assert!(rep.reconciled.contains(&"annotated") && !rep.incomplete.contains(&"annotated"));
    assert!(r.engine.store.remote_annotations(ArticleId(1)).unwrap().is_empty(), "removed elsewhere, listing complete");

    // Four annotated articles, one per page, two pages read: the listing is incomplete.
    let mut r = rig_with(SyncConfig { annotated_pages: 2, unread_fetch_pages: 4, ..Default::default() });
    for i in 1..=4u64 {
        r.fake.art(i, usec(i as i64), FEED);
        add_annotation(&r, i, i as i64);
    }
    r.fake.with(|s| s.page_cap = 1);
    let rep = r.full();
    assert!(rep.incomplete.contains(&"annotated"), "{rep:?}");
    // Article 4 is the oldest: beyond the two pages read, so its removal cannot be confirmed.
    r.fake.with(|s| s.arts.get_mut(&4).unwrap().annotations.clear());
    r.full();
    assert_eq!(r.engine.store.remote_annotations(ArticleId(4)).unwrap().len(), 1, "not cleared on an incomplete listing");
}

fn saved_summary(r: &Rig, id: u64) -> inoink::model::ArticleSummary {
    let q = ArticleQuery { collection: Some(Collection::Starred), limit: 100, visit_start_usec: 1, ..Default::default() };
    r.engine.store.query_articles(&q).unwrap().items.into_iter().find(|a| a.id == ArticleId(id)).expect("saved article listed")
}

#[test]
fn progress_is_refreshed_hourly_and_cleared_after_a_complete_listing() {
    let mut r = rig();
    seed(&r);
    r.fake.with(|s| {
        let a = s.arts.get_mut(&1).unwrap();
        a.starred_at = 1_700_000_000;
        a.progress = Some(serde_json::json!({"kind": "article", "length": 720, "progress": 40, "finished": false}));
    });
    let rep = r.full();
    assert!(rep.reconciled.contains(&"progress"), "{rep:?}");
    let a = saved_summary(&r, 1);
    assert_eq!((a.progress, a.finished, a.saved_at), (40, false, 1_700_000_000));
    assert_eq!(a.length_secs, 720);
    assert!(!a.new, "started elsewhere counts as seen");

    // An automatic cycle inside the hour does not ask again; after it, it does.
    r.fake.clear_log();
    r.clock.advance(901);
    r.engine.run_cycle(Reason::Wake);
    assert_eq!(r.fake.count("progress=1"), 0);
    r.clock.advance(3600);
    let rep = r.engine.run_cycle(Reason::Wake);
    assert!(rep.reconciled.contains(&"progress"), "{rep:?}");
    assert_eq!(r.fake.count("progress=1"), 1);

    // Unsaved elsewhere: a complete listing forgets the progress, the saved date stays.
    r.fake.with(|s| s.arts.get_mut(&1).unwrap().starred = false);
    r.full();
    assert!(r.engine.store.saved_progress(ArticleId(1)).unwrap().is_none());
}

#[test]
fn an_incomplete_progress_listing_keeps_what_it_did_not_reach() {
    let mut r = rig_with(SyncConfig { progress_pages: 1, ..Default::default() });
    seed(&r);
    r.fake.with(|s| {
        s.page_cap = 1;
        s.arts.get_mut(&10).unwrap().progress = Some(serde_json::json!({"kind": "article", "progress": 60, "finished": false}));
    });
    let rep = r.full();
    assert!(rep.incomplete.contains(&"progress"), "{rep:?}");
    assert_eq!(r.engine.store.saved_progress(ArticleId(10)).unwrap().map(|p| p.percent), Some(60));
}

#[test]
fn no_progress_request_without_saved_articles() {
    let mut r = rig();
    for i in 1..=2u64 {
        r.fake.art(i, usec(i as i64), FEED);
    }
    let rep = r.full();
    assert!(!rep.reconciled.contains(&"progress"), "{rep:?}");
    assert_eq!(r.fake.count("progress=1"), 0);
}

fn archived_ids(r: &Rig) -> Vec<u64> {
    let q = ArticleQuery { collection: Some(Collection::Archived), limit: 100, visit_start_usec: 1, ..Default::default() };
    let mut ids: Vec<u64> = r.engine.store.query_articles(&q).unwrap().items.into_iter().map(|a| a.id.0).collect();
    ids.sort();
    ids
}

#[test]
fn archive_listing_marks_and_fetches_archived() {
    let mut r = rig();
    seed(&r);
    r.fake.art(7, usec(7), FEED);
    r.fake.art(8, usec(8), FEED);
    r.fake.with(|s| {
        s.arts.get_mut(&7).unwrap().archived = true;
        let a = s.arts.get_mut(&8).unwrap();
        a.archived = true;
        a.progress = Some(serde_json::json!({"kind": "article", "length": 720, "progress": 60, "finished": false}));
    });
    let rep = r.full();
    assert_eq!(rep.error, None);
    assert!(rep.reconciled.contains(&"archive"), "{rep:?}");
    assert_eq!(archived_ids(&r), vec![7, 8]);
    assert!(r.eff(7, Membership::Archived) && !r.eff(7, Membership::Starred));
    assert_eq!(r.engine.store.saved_progress(ArticleId(8)).unwrap().map(|p| p.percent), Some(60));
}

#[test]
fn archive_listing_is_throttled_without_a_starred_change() {
    let mut r = rig();
    seed(&r);
    r.fake.art(7, usec(7), FEED);
    r.fake.with(|s| s.arts.get_mut(&7).unwrap().archived = true);
    r.full();
    let archive_requests = |r: &Rig| r.fake.log().iter().filter(|l| l.contains("items/ids") && l.contains("archived")).count();
    r.fake.clear_log();
    r.clock.advance(901);
    let rep = r.engine.run_cycle(Reason::Wake);
    assert!(!rep.reconciled.contains(&"archive"), "{rep:?}");
    assert_eq!(archive_requests(&r), 0);
    r.clock.advance(21_600);
    let rep = r.engine.run_cycle(Reason::Wake);
    assert!(rep.reconciled.contains(&"archive"), "{rep:?}");
    assert!(archive_requests(&r) >= 1);
}

#[test]
fn archived_elsewhere_moves_out_of_read_later_in_one_cycle() {
    let mut r = rig();
    seed(&r);
    r.full();
    assert!(r.eff(1, Membership::Starred));
    r.fake.with(|s| {
        let a = s.arts.get_mut(&1).unwrap();
        a.starred = false;
        a.archived = true;
    });
    r.clock.advance(901);
    let rep = r.engine.run_cycle(Reason::Wake);
    assert_eq!(rep.error, None);
    assert!(!r.eff(1, Membership::Starred) && r.eff(1, Membership::Archived), "{rep:?}");
    assert!(archived_ids(&r).contains(&1));
}

#[test]
fn saving_downloads_the_article_on_the_local_sync() {
    let mut r = rig();
    seed(&r);
    r.fake.art(11, OLD_TS, FEED);
    r.full();
    assert!(r.engine.store.has_article(ArticleId(11)).unwrap());
    assert!(!r.engine.store.download_state(ArticleId(11)).unwrap().readable_offline());
    r.engine.store.apply_intent(ArticleId(11), &Membership::Starred, true).unwrap();
    let rep = r.engine.run_cycle(Reason::Local);
    assert_eq!(rep.error, None);
    assert!(rep.cached >= 1);
    assert!(r.engine.store.download_state(ArticleId(11)).unwrap().readable_offline());
}
