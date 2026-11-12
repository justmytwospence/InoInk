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
use plato_core::input::{ButtonCode, ButtonStatus, DeviceEvent};
use plato_core::gesture::GestureEvent;
use plato_core::view::common::locate_by_id;
use plato_core::view::reader::Reader;
use plato_core::view::inoink::state::{InoInk, ReadingQueue, Tab};
use plato_core::view::ViewId;
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
    rx: mpsc::Receiver<Event>,
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
    Env { _dir: dir, context, tx, rx }
}

fn rows(home: &InoInkHome) -> Vec<u64> {
    home.children()[4].children().iter().filter_map(|c| c.downcast_ref::<ArticleRow>()).map(|r| r.item.id.0).collect()
}

fn send(home: &mut InoInkHome, env: &mut Env, e: Event) {
    let mut bus = VecDeque::new();
    let mut rq = RenderQueue::new();
    home.handle_event(&e, &env.tx, &mut bus, &mut rq, &mut env.context);
}

#[test]
fn opens_on_the_newsfeed_with_unread_only() {
    let mut e = env();
    assert_eq!(e.context.inoink.tab, Tab::Newsfeed);
    let mut rq = RenderQueue::new();
    let home = InoInkHome::new(e.context.fb.rect(), &e.tx, &mut rq, &mut e.context);
    assert_eq!(rows(&home), vec![4, 3, 2, 1], "every article is unread; newest first");
    assert_eq!(home.visible_articles().len(), 4);
}

#[test]
fn tab_and_category_are_remembered() {
    let mut e = env();
    e.context.inoink.open_collection(Collection::Folder("X".into()));
    drop(e.context.inoink.store.take());
    let s = Store::open(&e.context.inoink.data_dir.join("inoink.db"), "local").unwrap();
    assert!(s.get_meta("ui_tab").unwrap().unwrap().contains("Categories"));
    assert!(s.get_meta("ui_collection").unwrap().unwrap().contains("Folder"));
    assert_eq!(s.get_meta("ui_show_categories").unwrap().as_deref(), Some("0"));
}

#[test]
fn tapping_a_headline_in_the_newsfeed_saves_it() {
    let mut e = env();
    let mut rq = RenderQueue::new();
    let mut home = InoInkHome::new(e.context.fb.rect(), &e.tx, &mut rq, &mut e.context);
    send(&mut home, &mut e, Event::ArticleAction(ArticleId(3), ArticleAction::ToggleStar));
    assert!(e.context.inoink.store().unwrap().effective(ArticleId(3), &Membership::Starred).unwrap());
    assert_eq!(rows(&home), vec![4, 3, 2, 1], "it stays in the list");
    assert!(!e.context.inoink.store().unwrap().effective(ArticleId(3), &Membership::Read).unwrap(), "saving does not mark it read");
}

#[test]
fn read_later_shows_starred_newest_first() {
    let mut e = env();
    let mut rq = RenderQueue::new();
    let mut home = InoInkHome::new(e.context.fb.rect(), &e.tx, &mut rq, &mut e.context);
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::Tab(Tab::ReadLater))));
    assert_eq!(rows(&home), vec![2, 1]);
}

#[test]
fn opening_builds_a_reading_queue() {
    let mut e = env();
    let mut rq = RenderQueue::new();
    let mut home = InoInkHome::new(e.context.fb.rect(), &e.tx, &mut rq, &mut e.context);
    send(&mut home, &mut e, Event::OpenArticle(ArticleId(4)));
    let q = e.context.inoink.queue.clone().expect("a queue");
    assert_eq!(q.ids.iter().map(|i| i.0).collect::<Vec<_>>(), vec![4, 3, 2, 1]);
    assert_eq!(q.next_after(ArticleId(4)), Some(ArticleId(3)));
    assert_eq!(q.position(ArticleId(3)), Some((2, 4)));
    assert_eq!(q.next_after(ArticleId(1)), None);
    assert!(!q.read_later);
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::Tab(Tab::ReadLater))));
    send(&mut home, &mut e, Event::OpenArticle(ArticleId(2)));
    assert!(e.context.inoink.queue.clone().unwrap().read_later);
}

#[test]
fn end_of_unread_marks_only_seen_unstarred() {
    let mut e = env();
    let mut rq = RenderQueue::new();
    let mut home = InoInkHome::new(e.context.fb.rect(), &e.tx, &mut rq, &mut e.context);
    assert_eq!(rows(&home), vec![4, 3, 2, 1]);
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::MarkSeenRead)));
    // A new article arrives after the user saw the count and before they confirmed.
    e.context.inoink.store_mut().unwrap().upsert_article(&rec(5, 500_000_000, false), 0).unwrap();
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::ConfirmMarkSeenRead)));
    let st = e.context.inoink.store().unwrap();
    // 1 and 2 are starred in the fixtures and stay unread; 3 and 4 were seen and unstarred.
    assert!(st.effective(ArticleId(3), &Membership::Read).unwrap());
    assert!(st.effective(ArticleId(4), &Membership::Read).unwrap());
    assert!(!st.effective(ArticleId(1), &Membership::Read).unwrap());
    assert!(!st.effective(ArticleId(2), &Membership::Read).unwrap());
    assert!(!st.effective(ArticleId(5), &Membership::Read).unwrap(), "the newcomer was never seen");
}

#[test]
fn categories_list_folders_in_inoreader_order() {
    let mut e = env();
    {
        let st = e.context.inoink.store_mut().unwrap();
        let tag = |n: &str, sid: &str| inoink::store::TagInfo { id: format!("user/-/label/{n}"), kind: "folder".into(), sort_id: sid.into() };
        st.replace_tags(&[tag("Alpha", "00000001"), tag("Beta", "00000002")]).unwrap();
        st.set_stream_order(STATE_ROOT, "0000000200000001").unwrap();
    }
    e.context.inoink.set_tab(Tab::Categories);
    let mut rq = RenderQueue::new();
    let home = InoInkHome::new(e.context.fb.rect(), &e.tx, &mut rq, &mut e.context);
    let labels = home.category_labels();
    assert_eq!(&labels[..3], &["Folders".to_string(), "Beta".to_string(), "Alpha".to_string()]);
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

#[test]
fn end_card_finishes_and_requests_the_next_article() {
    let mut e = env();
    e.context.inoink.queue = Some(ReadingQueue { ids: vec![ArticleId(2), ArticleId(1)], read_later: true, label: "Read later".into() });
    let OpenResult::Ready(article) = open_article(&mut e.context, ArticleId(2)) else { panic!("cached article should open") };
    let mut reader = Reader::new_article(e.context.fb.rect(), &article, &e.tx, &mut e.context).expect("reader");
    let mut go = |reader: &mut Reader, e: &mut Env, evt: Event| {
        let mut bus = VecDeque::new();
        let mut rq = RenderQueue::new();
        plato_core::view::handle_event(reader, &evt, &e.tx, &mut bus, &mut rq, &mut e.context);
        while let Some(b) = bus.pop_front() {
            plato_core::view::handle_event(reader, &b, &e.tx, &mut VecDeque::new(), &mut rq, &mut e.context);
        }
    };
    go(&mut reader, &mut e, Event::Gesture(GestureEvent::HoldButtonShort(ButtonCode::Forward)));
    assert!(locate_by_id(&reader, ViewId::EndCard).is_some(), "holding forward shows the end card");

    // The release that ends the hold must not finish the article.
    go(&mut reader, &mut e, Event::Device(DeviceEvent::Button { time: 0.0, code: ButtonCode::Forward, status: ButtonStatus::Released }));
    assert!(!e.context.inoink.store().unwrap().effective(ArticleId(2), &Membership::Read).unwrap(), "the first release is ignored");
    assert!(locate_by_id(&reader, ViewId::EndCard).is_some());

    // The next press and release does (continue-on-swipe is on by default).
    go(&mut reader, &mut e, Event::Device(DeviceEvent::Button { time: 1.0, code: ButtonCode::Forward, status: ButtonStatus::Released }));
    let st = e.context.inoink.store().unwrap();
    assert!(st.effective(ArticleId(2), &Membership::Read).unwrap(), "Done marks it read");
    assert!(!st.effective(ArticleId(2), &Membership::Starred).unwrap(), "and removes the star");
    let mut next = None;
    while let Ok(evt) = e.rx.try_recv() {
        if let Event::OpenNext(id) = evt {
            next = Some(id);
        }
    }
    assert_eq!(next, Some(ArticleId(1)), "the next article of the list is requested");
}

fn drive(reader: &mut Reader, e: &mut Env, evt: Event) {
    let mut bus = VecDeque::new();
    let mut rq = RenderQueue::new();
    plato_core::view::handle_event(reader, &evt, &e.tx, &mut bus, &mut rq, &mut e.context);
    while let Some(b) = bus.pop_front() {
        plato_core::view::handle_event(reader, &b, &e.tx, &mut VecDeque::new(), &mut rq, &mut e.context);
    }
}

fn requested_next(e: &Env) -> Option<ArticleId> {
    let mut next = None;
    while let Ok(evt) = e.rx.try_recv() {
        if let Event::OpenNext(id) = evt {
            next = Some(id);
        }
    }
    next
}

#[test]
fn swiping_forward_on_the_end_card_marks_read_and_continues_by_default() {
    let mut e = env();
    e.context.inoink.queue = Some(ReadingQueue { ids: vec![ArticleId(4), ArticleId(3)], read_later: false, label: "Newsfeed".into() });
    let OpenResult::Ready(article) = open_article(&mut e.context, ArticleId(4)) else { panic!("cached article should open") };
    let mut reader = Reader::new_article(e.context.fb.rect(), &article, &e.tx, &mut e.context).expect("reader");
    drive(&mut reader, &mut e, Event::Update(plato_core::framebuffer::UpdateMode::Partial));
    // The fixture article fits on one page: the next forward swipe shows the card, the one after continues.
    let swipe = |x0: i32, x1: i32| Event::Gesture(GestureEvent::Swipe { dir: if x1 < x0 { plato_core::geom::Dir::West } else { plato_core::geom::Dir::East }, start: plato_core::geom::Point::new(x0, 800), end: plato_core::geom::Point::new(x1, 800) });
    drive(&mut reader, &mut e, swipe(1000, 200));
    assert!(locate_by_id(&reader, ViewId::EndCard).is_some());
    drive(&mut reader, &mut e, swipe(1000, 200));
    assert!(e.context.inoink.store().unwrap().effective(ArticleId(4), &Membership::Read).unwrap(), "marked read");
    assert_eq!(requested_next(&e), Some(ArticleId(3)));
}

#[test]
fn swiping_back_on_the_first_page_opens_the_previous_article_at_its_start() {
    let mut e = env();
    e.context.inoink.queue = Some(ReadingQueue { ids: vec![ArticleId(4), ArticleId(3)], read_later: false, label: "Newsfeed".into() });
    let OpenResult::Ready(article) = open_article(&mut e.context, ArticleId(3)) else { panic!("cached article should open") };
    let mut reader = Reader::new_article(e.context.fb.rect(), &article, &e.tx, &mut e.context).expect("reader");
    drive(&mut reader, &mut e, Event::Update(plato_core::framebuffer::UpdateMode::Partial));
    let back = Event::Gesture(GestureEvent::Swipe { dir: plato_core::geom::Dir::East, start: plato_core::geom::Point::new(200, 800), end: plato_core::geom::Point::new(1000, 800) });
    drive(&mut reader, &mut e, back);
    assert_eq!(requested_next(&e), Some(ArticleId(4)), "the previous article of the list");
    assert!(e.context.inoink.open_at_start);
    assert!(!reader.children().iter().any(|c| c.downcast_ref::<plato_core::view::notification::Notification>().is_some()), "no popup");
}
