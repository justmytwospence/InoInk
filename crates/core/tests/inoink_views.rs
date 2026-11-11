//! Headless view and action behaviour: defaults, Done versus Mark read, undo, frozen bulk
//! selections and failure handling. Run through scripts/container-run.sh (links MuPDF).

use inoink::cache::{Cache, CacheConfig, Control};
use inoink::content::images::{Fetched, Fetcher};
use inoink::model::*;
use inoink::store::{ArticleRecord, Store};
use plato_core::battery::FakeBattery;
use plato_core::context::Context;
use plato_core::font::Fonts;
use plato_core::frontlight::LightLevels;
use plato_core::framebuffer::Pixmap;
use plato_core::settings::Settings;
use plato_core::view::inoink::actions::*;
use plato_core::view::inoink::home::InoInkHome;
use plato_core::view::inoink::row::ArticleRow;
use plato_core::view::inoink::state::InoInk;
use plato_core::view::{EntryId, Event, RenderQueue, View};
use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc};

struct NoNet;
impl Fetcher for NoNet {
    fn get(&self, _u: &url::Url, _m: usize) -> inoink::Result<Fetched> {
        Ok(Fetched { status: 404, ..Default::default() })
    }
}

fn rec(id: u64, ts: i64, starred: bool) -> ArticleRecord {
    let mut cats = vec!["user/1/state/com.google/reading-list".to_string()];
    if starred {
        cats.push("user/1/state/com.google/starred".into());
    }
    ArticleRecord {
        id,
        title: format!("Article {id}"),
        origin_stream: "feed/https://x.example/rss".into(),
        origin_title: "X".into(),
        canonical_url: format!("https://x.example/{id}"),
        timestamp_usec: ts,
        published: ts / 1_000_000,
        summary_html: format!("<p>Body {id}</p>"),
        raw_json: "{}".into(),
        categories: cats,
        ..Default::default()
    }
}

struct Env {
    _dir: tempfile::TempDir,
    context: Context,
    tx: mpsc::Sender<Event>,
    _rx: mpsc::Receiver<Event>,
}

fn env() -> Env {
    std::env::set_current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    {
        let mut store = Store::open(&dir.path().join("inoink.db"), "local").unwrap();
        let cache = Cache::new(&dir.path().join("cache"), "local", CacheConfig::default());
        let ctl = Control::new(Arc::new(AtomicBool::new(false)));
        for (id, ts, starred) in [(1u64, 100i64, true), (2, 200, true), (3, 300, false), (4, 400, false)] {
            store.upsert_article(&rec(id, ts * 1_000_000, starred), 0).unwrap();
            cache.cache_article(&mut store, &NoNet, ArticleId(id), &ctl).unwrap();
        }
        store.checkpoint().unwrap();
    }
    let mut settings = Settings::default();
    settings.inoink.data_dir = dir.path().to_path_buf();
    let (tx, rx) = mpsc::channel();
    let inoink = InoInk::open(&settings.inoink, tx.clone(), false).unwrap();
    let fb = Box::new(Pixmap::new(1264, 1680, 3));
    let context = Context::new(fb, None, inoink, settings, Fonts::load().unwrap(), Box::new(FakeBattery::new()), Box::new(LightLevels::default()), Box::new(0u16));
    Env { _dir: dir, context, tx, _rx: rx }
}

fn rows(home: &InoInkHome) -> Vec<u64> {
    home.children()[2].children().iter().filter_map(|c| c.downcast_ref::<ArticleRow>()).map(|r| r.item.id.0).collect()
}

fn send(home: &mut InoInkHome, env: &mut Env, e: Event) {
    let mut bus = VecDeque::new();
    let mut rq = RenderQueue::new();
    home.handle_event(&e, &env.tx, &mut bus, &mut rq, &mut env.context);
}

#[test]
fn opens_on_starred_by_default_and_lists_newest_first() {
    let mut e = env();
    assert_eq!(e.context.inoink.collection, Collection::Starred);
    let mut rq = RenderQueue::new();
    let home = InoInkHome::new(e.context.fb.rect(), &e.tx, &mut rq, &mut e.context);
    assert_eq!(rows(&home), vec![2, 1], "only starred articles, newest first");
}

#[test]
fn collection_and_filter_are_remembered() {
    let mut e = env();
    e.context.inoink.set_view(Collection::Unread, true);
    drop(e.context.inoink.store.take());
    let s = Store::open(&e.context.inoink.data_dir.join("inoink.db"), "local").unwrap();
    assert!(s.get_meta("ui_collection").unwrap().unwrap().contains("Unread"));
    assert_eq!(s.get_meta("ui_unread_only").unwrap().as_deref(), Some("1"));
}

#[test]
fn done_marks_read_and_unstars_and_can_be_undone_while_mark_read_keeps_the_star() {
    let mut e = env();
    let out = apply_article_action(&mut e.context, ArticleId(1), &ArticleAction::Done).unwrap();
    assert!(out.close);
    let st = e.context.inoink.store().unwrap();
    assert!(st.effective(ArticleId(1), &Membership::Read).unwrap());
    assert!(!st.effective(ArticleId(1), &Membership::Starred).unwrap());
    assert!(e.context.inoink.last_undo.is_some());

    undo_done(&mut e.context).unwrap();
    let st = e.context.inoink.store().unwrap();
    assert!(!st.effective(ArticleId(1), &Membership::Read).unwrap());
    assert!(st.effective(ArticleId(1), &Membership::Starred).unwrap());

    apply_article_action(&mut e.context, ArticleId(2), &ArticleAction::ToggleRead).unwrap();
    let st = e.context.inoink.store().unwrap();
    assert!(st.effective(ArticleId(2), &Membership::Read).unwrap());
    assert!(st.effective(ArticleId(2), &Membership::Starred).unwrap(), "Mark read preserves the star");
}

#[test]
fn opening_does_not_mark_read_and_leases_the_version() {
    let mut e = env();
    match open_article(&mut e.context, ArticleId(3)) {
        OpenResult::Ready(a) => {
            assert!(a.path.is_file());
            assert!(e.context.inoink.store().unwrap().is_leased(ArticleId(3)).unwrap());
            release_article(&mut e.context, a.id, a.version);
            assert!(!e.context.inoink.store().unwrap().is_leased(ArticleId(3)).unwrap());
        }
        _ => panic!("cached article should open"),
    }
    assert!(!e.context.inoink.store().unwrap().effective(ArticleId(3), &Membership::Read).unwrap());
    // An article that was never downloaded cannot open offline.
    e.context.inoink.store_mut().unwrap().upsert_article(&rec(9, 900_000_000, false), 0).unwrap();
    assert!(matches!(open_article(&mut e.context, ArticleId(9)), OpenResult::Unavailable(_)));
}

#[test]
fn a_failed_save_changes_nothing_and_says_so() {
    let mut e = env();
    e.context.inoink.usb_share(); // the database is closed while storage is shared
    let r = apply_article_action(&mut e.context, ArticleId(1), &ArticleAction::ToggleStar);
    assert_eq!(r.unwrap_err(), "Could not save change");
    e.context.inoink.usb_unshare();
    assert!(e.context.inoink.store().unwrap().effective(ArticleId(1), &Membership::Starred).unwrap());
}

#[test]
fn marking_a_list_read_uses_a_frozen_selection() {
    let mut e = env();
    e.context.inoink.set_view(Collection::Unread, false);
    let mut rq = RenderQueue::new();
    let mut home = InoInkHome::new(e.context.fb.rect(), &e.tx, &mut rq, &mut e.context);
    assert_eq!(rows(&home), vec![4, 3, 2, 1]);
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::MarkListRead)));
    // A new article arrives after the user saw the count and before they confirmed.
    e.context.inoink.store_mut().unwrap().upsert_article(&rec(5, 500_000_000, false), 0).unwrap();
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::ConfirmMarkListRead)));
    let st = e.context.inoink.store().unwrap();
    for id in 1..=4 {
        assert!(st.effective(ArticleId(id), &Membership::Read).unwrap(), "#{id} in the frozen scope");
    }
    assert!(!st.effective(ArticleId(5), &Membership::Read).unwrap(), "the newcomer is untouched");
    assert!(st.effective(ArticleId(1), &Membership::Starred).unwrap(), "stars are kept");
}

#[test]
fn tags_are_added_and_removed_individually() {
    let mut e = env();
    apply_article_action(&mut e.context, ArticleId(3), &ArticleAction::AddTag("later".into())).unwrap();
    apply_article_action(&mut e.context, ArticleId(3), &ArticleAction::AddTag("work".into())).unwrap();
    apply_article_action(&mut e.context, ArticleId(3), &ArticleAction::RemoveTag("later".into())).unwrap();
    assert_eq!(e.context.inoink.store().unwrap().effective_labels(ArticleId(3)).unwrap(), vec!["work".to_string()]);
    assert!(apply_article_action(&mut e.context, ArticleId(3), &ArticleAction::AddTag("  ".into())).is_err());
}
