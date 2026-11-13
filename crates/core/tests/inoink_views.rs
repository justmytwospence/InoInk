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
use plato_core::geom::{CycleDir, Dir};
use plato_core::settings::{ButtonScheme, QueueTitles};
use plato_core::view::inoink::article_bar::ArticleBar;
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
    let mut home = InoInkHome::new(e.context.fb.rect(), &e.tx, &mut rq, &mut e.context);
    let labels = home.category_labels();
    assert_eq!(&labels[..3], &["Folders".to_string(), "Beta".to_string(), "Alpha".to_string()]);
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::CategorySort(CategorySort::Name))));
    assert_eq!(&home.category_labels()[..3], &["Folders".to_string(), "Alpha".to_string(), "Beta".to_string()]);
    assert_eq!(e.context.inoink.category_sort(), CategorySort::Name, "the choice is remembered");
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

fn sent(e: &Env) -> Vec<Event> {
    let mut out = Vec::new();
    while let Ok(evt) = e.rx.try_recv() {
        out.push(evt);
    }
    out
}

fn opens(evts: &[Event]) -> Vec<ArticleId> {
    evts.iter().filter_map(|e| if let Event::OpenNext(id) = e { Some(*id) } else { None }).collect()
}

fn swipe_v(y0: i32, y1: i32) -> Event {
    Event::Gesture(GestureEvent::Swipe { dir: if y1 < y0 { Dir::North } else { Dir::South }, start: plato_core::geom::Point::new(632, y0), end: plato_core::geom::Point::new(632, y1) })
}

fn swipe_h(x0: i32, x1: i32) -> Event {
    Event::Gesture(GestureEvent::Swipe { dir: if x1 < x0 { Dir::West } else { Dir::East }, start: plato_core::geom::Point::new(x0, 800), end: plato_core::geom::Point::new(x1, 800) })
}

fn open(e: &mut Env, ids: &[u64], read_later: bool, id: u64) -> Reader {
    e.context.inoink.queue = Some(ReadingQueue { ids: ids.iter().map(|i| ArticleId(*i)).collect(), read_later, label: if read_later { "Read later" } else { "Newsfeed" }.into() });
    let OpenResult::Ready(article) = open_article(&mut e.context, ArticleId(id)) else { panic!("cached article should open") };
    let mut reader = Reader::new_article(e.context.fb.rect(), &article, &e.tx, &mut e.context).expect("reader");
    drive(&mut reader, e, Event::Update(plato_core::framebuffer::UpdateMode::Partial));
    reader
}

fn press(code: ButtonCode, status: ButtonStatus) -> Event {
    Event::Device(DeviceEvent::Button { time: 0.0, code, status })
}

#[test]
fn paging_past_the_last_page_opens_the_next_article_without_marking_read() {
    let mut e = env();
    let mut reader = open(&mut e, &[2, 1], true, 2);
    drive(&mut reader, &mut e, swipe_h(1000, 200));
    assert_eq!(opens(&sent(&e)), vec![ArticleId(1)]);
    let st = e.context.inoink.store().unwrap();
    assert!(st.effective(ArticleId(2), &Membership::Starred).unwrap(), "still saved for later");
    assert!(!st.effective(ArticleId(2), &Membership::Read).unwrap(), "not marked read");
    assert!(locate_by_id(&reader, ViewId::QueueNav).is_none());
}

#[test]
fn holding_forward_opens_the_next_article_on_release() {
    let mut e = env();
    let mut reader = open(&mut e, &[4, 3], false, 4);
    sent(&e);
    drive(&mut reader, &mut e, Event::Gesture(GestureEvent::HoldButtonShort(ButtonCode::Forward)));
    assert!(opens(&sent(&e)).is_empty());
    drive(&mut reader, &mut e, press(ButtonCode::Forward, ButtonStatus::Released));
    assert_eq!(opens(&sent(&e)), vec![ArticleId(3)]);
}

#[test]
fn holding_back_opens_the_previous_article() {
    let mut e = env();
    let mut reader = open(&mut e, &[4, 3], false, 3);
    drive(&mut reader, &mut e, Event::Gesture(GestureEvent::HoldButtonShort(ButtonCode::Backward)));
    drive(&mut reader, &mut e, press(ButtonCode::Backward, ButtonStatus::Released));
    assert_eq!(opens(&sent(&e)), vec![ArticleId(4)]);
    assert!(!e.context.inoink.open_at_end);
}

#[test]
fn swiping_up_opens_the_next_article() {
    let mut e = env();
    let mut reader = open(&mut e, &[4, 3], false, 4);
    drive(&mut reader, &mut e, swipe_v(1400, 600));
    assert_eq!(opens(&sent(&e)), vec![ArticleId(3)]);
    assert!(!e.context.inoink.store().unwrap().effective(ArticleId(4), &Membership::Read).unwrap());
}

#[test]
fn a_short_vertical_swipe_does_nothing() {
    let mut e = env();
    let mut reader = open(&mut e, &[4, 3], false, 4);
    sent(&e);
    drive(&mut reader, &mut e, swipe_v(900, 700));
    assert!(sent(&e).iter().all(|evt| !matches!(evt, Event::OpenNext(_) | Event::Back)));
}

#[test]
fn swiping_down_returns_to_the_previous_article_where_it_was_left() {
    let mut e = env();
    let mut reader = open(&mut e, &[4, 3], false, 3);
    drive(&mut reader, &mut e, swipe_v(600, 1400));
    assert_eq!(opens(&sent(&e)), vec![ArticleId(4)]);
    assert!(!e.context.inoink.open_at_end);
}

#[test]
fn swiping_down_on_the_first_article_says_so() {
    let mut e = env();
    let mut reader = open(&mut e, &[4, 3], false, 4);
    drive(&mut reader, &mut e, swipe_v(600, 1400));
    assert!(opens(&sent(&e)).is_empty());
    assert!(reader.children().iter().any(|c| c.downcast_ref::<plato_core::view::notification::Notification>().is_some()));
}

#[test]
fn after_the_last_article_you_are_back_at_the_list() {
    let mut e = env();
    let mut reader = open(&mut e, &[4, 3], false, 3);
    drive(&mut reader, &mut e, swipe_v(1400, 600));
    let evts = sent(&e);
    assert!(opens(&evts).is_empty());
    assert!(evts.iter().any(|e| matches!(e, Event::Back)));
}

#[test]
fn a_second_request_while_the_first_opens_is_ignored() {
    let mut e = env();
    let mut reader = open(&mut e, &[4, 3, 2], false, 4);
    drive(&mut reader, &mut e, swipe_v(1400, 600));
    drive(&mut reader, &mut e, swipe_v(1400, 600));
    assert_eq!(opens(&sent(&e)), vec![ArticleId(3)]);
}

#[test]
fn the_bar_names_the_neighbours_and_tapping_one_opens_it() {
    let mut e = env();
    assert_eq!(Settings::default().inoink.queue_titles, QueueTitles::WithBar);
    let mut reader = open(&mut e, &[4, 3, 2], false, 3);
    drive(&mut reader, &mut e, Event::Gesture(GestureEvent::Tap(plato_core::geom::Point::new(632, 840))));
    let index = locate_by_id(&reader, ViewId::QueueNav).expect("titles row");
    let r = *reader.children()[index].rect();
    let y = (r.min.y + r.max.y) / 2;
    sent(&e);
    drive(&mut reader, &mut e, Event::Gesture(GestureEvent::Tap(plato_core::geom::Point::new(r.max.x - r.width() as i32 / 8, y))));
    assert_eq!(opens(&sent(&e)), vec![ArticleId(2)]);
}

#[test]
fn no_titles_row_without_neighbours() {
    let mut e = env();
    let mut reader = open(&mut e, &[4], false, 4);
    drive(&mut reader, &mut e, Event::Gesture(GestureEvent::Tap(plato_core::geom::Point::new(632, 840))));
    assert!(locate_by_id(&reader, ViewId::QueueNav).is_none());
}

#[test]
fn swiping_back_on_the_first_page_opens_the_previous_article_on_its_last_page() {
    let mut e = env();
    e.context.inoink.queue = Some(ReadingQueue { ids: vec![ArticleId(4), ArticleId(3)], read_later: false, label: "Newsfeed".into() });
    let OpenResult::Ready(article) = open_article(&mut e.context, ArticleId(3)) else { panic!("cached article should open") };
    let mut reader = Reader::new_article(e.context.fb.rect(), &article, &e.tx, &mut e.context).expect("reader");
    drive(&mut reader, &mut e, Event::Update(plato_core::framebuffer::UpdateMode::Partial));
    let back = Event::Gesture(GestureEvent::Swipe { dir: plato_core::geom::Dir::East, start: plato_core::geom::Point::new(200, 800), end: plato_core::geom::Point::new(1000, 800) });
    drive(&mut reader, &mut e, back);
    assert_eq!(requested_next(&e), Some(ArticleId(4)), "the previous article of the list");
    assert!(e.context.inoink.open_at_end);
    let OpenResult::Ready(prev) = open_article(&mut e.context, ArticleId(4)) else { panic!("cached article should open") };
    assert_eq!(prev.offset, Some(usize::MAX), "opens on the last page");
    assert!(!e.context.inoink.open_at_end, "the flag is taken");
    assert!(!reader.children().iter().any(|c| c.downcast_ref::<plato_core::view::notification::Notification>().is_some()), "no popup");
}

/// Adds articles 100..140 (every third one read) and pages through the list: every page must
/// report the same total, and the last page must be the total.
fn pages_are_counted(group: Option<ArticleGroup>) {
    let mut e = env();
    {
        let st = e.context.inoink.store_mut().unwrap();
        for id in 100u64..140 {
            st.upsert_article(&rec(id, id as i64 * 1_000_000, false), 0).unwrap();
            if id % 3 == 0 {
                st.apply_intent(ArticleId(id), &Membership::Read, true).unwrap();
            }
        }
    }
    let mut rq = RenderQueue::new();
    let mut home = InoInkHome::new(e.context.fb.rect(), &e.tx, &mut rq, &mut e.context);
    if let Some(g) = group {
        send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::UnreadOnly(false))));
        send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::ViewGroup(g))));
    }
    let (first, total, _) = home.page_status();
    assert_eq!(first, 1);
    assert!(total > 1, "40 articles take several pages");
    let mut seen = 1;
    loop {
        send(&mut home, &mut e, Event::Page(CycleDir::Next));
        let (page, t, _) = home.page_status();
        assert_eq!(t, total, "page {page}");
        if page == seen {
            break;
        }
        seen = page;
    }
    assert_eq!(seen, total, "the last page is the total");
}

#[test]
fn list_pages_are_counted_exactly() {
    pages_are_counted(None);
}

#[test]
fn grouped_list_pages_are_counted_exactly() {
    pages_are_counted(Some(ArticleGroup::Unread));
}

#[test]
fn categories_show_their_page_count() {
    let mut e = env();
    e.context.inoink.set_tab(Tab::Categories);
    let mut rq = RenderQueue::new();
    let home = InoInkHome::new(e.context.fb.rect(), &e.tx, &mut rq, &mut e.context);
    assert_eq!(home.page_status(), (1, 1, false));
}

fn add_long_article(e: &mut Env, id: u64) {
    let mut r = rec(id, 900_000_000 + id as i64, false);
    r.summary_html = "<p>Paragraph of a long article for paging tests.</p>".repeat(300);
    let ino = &mut e.context.inoink;
    let store = ino.store.as_mut().unwrap();
    store.upsert_article(&r, 0).unwrap();
    ino.cache.cache_article(store, &NoNet, ArticleId(id), &Control::new(Arc::new(AtomicBool::new(false)))).unwrap();
}

fn has_bar(reader: &Reader) -> bool {
    reader.children().iter().any(|c| c.downcast_ref::<ArticleBar>().is_some())
}

#[test]
fn an_article_opened_mid_way_shows_a_running_header_until_the_first_page_turn() {
    let mut e = env();
    add_long_article(&mut e, 9);
    e.context.inoink.open_at_end = true;
    let mut reader = open(&mut e, &[9], false, 9);
    assert!(!has_bar(&reader));
    assert_eq!(reader.running_header(), Some("Article 9"));
    drive(&mut reader, &mut e, Event::Gesture(GestureEvent::Tap(plato_core::geom::Point::new(100, 840))));
    assert!(reader.running_header().is_none());
}

#[test]
fn an_article_opened_at_its_start_shows_no_bar() {
    let mut e = env();
    let reader = open(&mut e, &[4, 3], false, 4);
    assert!(!has_bar(&reader));
    assert!(reader.running_header().is_none());
}

#[test]
fn with_inverted_buttons_swiping_down_opens_the_next_article() {
    let mut e = env();
    e.context.settings.button_scheme = ButtonScheme::Inverted;
    let mut reader = open(&mut e, &[4, 3], false, 4);
    drive(&mut reader, &mut e, swipe_v(600, 1400));
    assert_eq!(opens(&sent(&e)), vec![ArticleId(3)]);
}

#[test]
fn a_slanted_upward_swipe_still_opens_the_next_article() {
    let mut e = env();
    let mut reader = open(&mut e, &[4, 3], false, 4);
    let p = plato_core::geom::Point::new;
    drive(&mut reader, &mut e, Event::Gesture(GestureEvent::SlantedSwipe { dir: plato_core::geom::DiagDir::NorthEast, start: p(500, 1400), end: p(800, 700) }));
    assert_eq!(opens(&sent(&e)), vec![ArticleId(3)]);
}

#[test]
fn a_stalled_download_blocks_briefly_then_gives_way() {
    let mut e = env();
    let mut reader = open(&mut e, &[4, 3], false, 4);
    e.context.inoink.opening = Some((ArticleId(9), plato_core::view::inoink::state::OpenMode::Replace));
    e.context.inoink.opening_at = Some(std::time::Instant::now());
    sent(&e);
    drive(&mut reader, &mut e, swipe_v(1400, 600));
    assert!(opens(&sent(&e)).is_empty());
    assert!(reader.children().iter().any(|c| c.downcast_ref::<plato_core::view::notification::Notification>().is_some()));

    let mut reader = open(&mut e, &[4, 3], false, 4);
    e.context.inoink.opening = Some((ArticleId(9), plato_core::view::inoink::state::OpenMode::Replace));
    e.context.inoink.opening_at = std::time::Instant::now().checked_sub(std::time::Duration::from_secs(11));
    sent(&e);
    drive(&mut reader, &mut e, swipe_v(1400, 600));
    assert_eq!(opens(&sent(&e)), vec![ArticleId(3)]);
    assert!(e.context.inoink.opening.is_none());
}

#[test]
fn no_prefetch_without_a_sync_worker() {
    let mut e = env();
    let _reader = open(&mut e, &[4, 3, 2], false, 3);
    assert!(e.context.inoink.prefetched.is_empty());
}
