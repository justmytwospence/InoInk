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
use plato_core::view::inoink::row::{ArticleRow, RowMode};
use plato_core::geom::{Point, Rectangle};
use plato_core::gesture::GestureEvent as Gest;
use inoink::store::ListSpan;
use plato_core::view::EntryKind;
use plato_core::input::{ButtonCode, ButtonStatus, DeviceEvent};
use plato_core::geom::{CycleDir, Dir};
use plato_core::settings::{AfterEnd, ButtonScheme, QueueTitles};
use plato_core::view::inoink::article_end::ArticleEnd;
use plato_core::view::inoink::article_bar::{ArticleBar, CommentsButton};
use plato_core::gesture::GestureEvent;
use plato_core::view::common::locate_by_id;
use plato_core::view::reader::Reader;
use plato_core::view::inoink::state::{InoInk, ReadingQueue, Tab, Undo};
use plato_core::view::ViewId;
use plato_core::view::common::locate;
use plato_core::view::notification::Notification;
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

fn has_undo(e: &Env) -> bool {
    plato_core::view::inoink::manage::main_menu_entries(&e.context).into_iter().any(|k| matches!(k, EntryKind::Command(l, _) if l == "Undo last change"))
}

fn send(home: &mut InoInkHome, env: &mut Env, e: Event) {
    let mut bus = VecDeque::new();
    let mut rq = RenderQueue::new();
    home.handle_event(&e, &env.tx, &mut bus, &mut rq, &mut env.context);
}

#[test]
fn sandbox_refuses_online_and_ignores_local_credentials_and_sharing() {
    let e = env();
    let dir = &e.context.inoink.data_dir;
    std::fs::write(dir.join("preview-sandbox.json"), "not json").unwrap();
    std::fs::write(dir.join("credentials.json"), "invalid").unwrap();
    std::fs::write(dir.join("share.json"), "invalid").unwrap();
    let error = match InoInk::open(&e.context.settings.inoink, e.tx.clone(), true) {
        Err(error) => error,
        Ok(_) => panic!("sandbox must reject online startup"),
    };
    assert_eq!(error, "This preview sandbox is offline-only.");
    assert!(!dir.join("device.key").exists());
    let offline = InoInk::open(&e.context.settings.inoink, e.tx.clone(), false).unwrap();
    assert!(offline.sandbox);
    assert_eq!(offline.account, "local");
    assert!(offline.handle.is_none());
    assert!(offline.share.karakeep.is_none());

    // Valid credentials for a different account must also be ignored.
    std::fs::write(dir.join("credentials.json"), r#"{"client_id":"x","client_secret":"x","redirect_uri":"https://example.com","access_token":"x","refresh_token":"x","expires_at":0,"scope":"read","user_id":"other"}"#).unwrap();
    std::fs::write(dir.join("share.json"), r#"{"karakeep":{"url":"https://example.com","api_key":"x"}}"#).unwrap();
    let offline = InoInk::open(&e.context.settings.inoink, e.tx.clone(), false).unwrap();
    assert_eq!(offline.account, "local");
    assert!(offline.handle.is_none());
    assert!(offline.share.karakeep.is_none());
    assert!(offline.store().unwrap().effective(ArticleId(1), &Membership::Starred).unwrap());
}

#[test]
fn sandbox_actions_persist_locally_and_sync_stays_offline() {
    let mut e = env();
    std::fs::write(e.context.inoink.data_dir.join("preview-sandbox.json"), "{}").unwrap();
    e.context.inoink = InoInk::open(&e.context.settings.inoink, e.tx.clone(), false).unwrap();
    let mut rq = RenderQueue::new();
    let mut home = InoInkHome::new(e.context.fb.rect(), &e.tx, &mut rq, &mut e.context);
    apply_article_action(&mut e.context, ArticleId(1), &ArticleAction::Done).unwrap();
    undo_done(&mut e.context).unwrap();
    assert!(!e.context.inoink.store().unwrap().effective(ArticleId(1), &Membership::Read).unwrap());
    assert!(e.context.inoink.store().unwrap().effective(ArticleId(1), &Membership::Starred).unwrap());
    apply_article_action(&mut e.context, ArticleId(1), &ArticleAction::Done).unwrap();
    apply_article_action(&mut e.context, ArticleId(3), &ArticleAction::ToggleStar).unwrap();
    apply_article_action(&mut e.context, ArticleId(2), &ArticleAction::ToggleRead).unwrap();
    apply_bulk_action(&mut e.context, &[ArticleId(4)], BulkAction::MarkRead).unwrap();
    undo_done(&mut e.context).unwrap();
    assert!(!e.context.inoink.store().unwrap().effective(ArticleId(4), &Membership::Read).unwrap());
    assert_eq!(e.context.inoink.status_text(), "Snapshot sandbox · local changes only");
    assert!(!e.context.inoink.status_text().contains("to send"));
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::SyncNow)));
    assert!(locate::<Notification>(&home).is_some());
    send(&mut home, &mut e, Event::Reseed);
    assert!(e.context.inoink.handle.is_none());
    e.context.inoink = InoInk::open(&e.context.settings.inoink, e.tx.clone(), false).unwrap();
    assert!(e.context.inoink.handle.is_none());
    let store = e.context.inoink.store().unwrap();
    assert!(store.effective(ArticleId(1), &Membership::Read).unwrap());
    assert!(!store.effective(ArticleId(1), &Membership::Starred).unwrap());
    assert!(store.effective(ArticleId(2), &Membership::Read).unwrap());
    assert!(store.effective(ArticleId(3), &Membership::Starred).unwrap());
    assert!(!store.effective(ArticleId(4), &Membership::Read).unwrap());
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
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::Select(SelectPreset::SeenUnmarked))));
    assert_eq!(home.selected_articles(), Some(vec![ArticleId(3), ArticleId(4)]));
    // A new article arrives after the selection was made and before Read.
    e.context.inoink.store_mut().unwrap().upsert_article(&rec(5, 500_000_000, false), 0).unwrap();
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::BulkApply(BulkAction::MarkRead))));
    assert_eq!(home.selected_articles(), None, "applying leaves selection mode");
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
    assert!(e.context.inoink.store().unwrap().seen_at(ArticleId(3)).unwrap().is_some(), "opening counts as seen");
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
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::Select(SelectPreset::List))));
    assert_eq!(home.selected_articles().map(|v| v.len()), Some(4));
    // A new article arrives after the selection was made and before Read.
    e.context.inoink.store_mut().unwrap().upsert_article(&rec(5, 500_000_000, false), 0).unwrap();
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::BulkApply(BulkAction::MarkRead))));
    let st = e.context.inoink.store().unwrap();
    for id in 1..=4 {
        assert!(st.effective(ArticleId(id), &Membership::Read).unwrap(), "#{id} in the frozen scope");
    }
    assert!(!st.effective(ArticleId(5), &Membership::Read).unwrap(), "the newcomer is untouched");
    assert!(st.effective(ArticleId(1), &Membership::Starred).unwrap(), "stars are kept");
}

fn new_rows(home: &InoInkHome) -> Vec<(u64, bool)> {
    home.children()[4].children().iter().filter_map(|c| c.downcast_ref::<ArticleRow>()).map(|r| (r.item.id.0, r.item.new)).collect()
}

#[test]
fn new_marks_last_for_the_visit() {
    let mut e = env();
    let mut rq = RenderQueue::new();
    let mut home = InoInkHome::new(e.context.fb.rect(), &e.tx, &mut rq, &mut e.context);
    assert_eq!(new_rows(&home), vec![(4, true), (3, true), (2, true), (1, true)], "never seen before this visit");
    // Changing the view (or a sync refresh) reloads the list without starting a new visit.
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::ViewSort(ArticleSort::Oldest))));
    assert_eq!(new_rows(&home), vec![(1, true), (2, true), (3, true), (4, true)], "still new while the visit lasts");
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::ViewSort(ArticleSort::Newest))));
    std::thread::sleep(std::time::Duration::from_millis(2));
    e.context.inoink.store_mut().unwrap().upsert_article(&rec(5, 500_000_000, false), 0).unwrap();
    // Back from an article (or awake again): a new visit.
    send(&mut home, &mut e, Event::Reseed);
    assert_eq!(new_rows(&home), vec![(5, true), (4, false), (3, false), (2, false), (1, false)]);
}

#[test]
fn selection_toggles_ranges_and_applies_with_undo() {
    let mut e = env();
    let mut rq = RenderQueue::new();
    let mut home = InoInkHome::new(e.context.fb.rect(), &e.tx, &mut rq, &mut e.context);
    let sel = |home: &mut InoInkHome, e: &mut Env, entry: InoInkEntry| send(home, e, Event::Select(EntryId::InoInk(entry)));
    let children = home.children().len();
    assert!(!has_undo(&e));
    sel(&mut home, &mut e, InoInkEntry::Select(SelectPreset::None));
    assert_eq!(home.selected_articles(), Some(vec![]));
    assert_eq!(rows(&home), vec![4, 3, 2, 1]);
    sel(&mut home, &mut e, InoInkEntry::ToggleSelected(ArticleId(4)));
    sel(&mut home, &mut e, InoInkEntry::SelectRangeTo(ArticleId(2)));
    assert_eq!(home.selected_articles(), Some(vec![ArticleId(2), ArticleId(3), ArticleId(4)]));
    sel(&mut home, &mut e, InoInkEntry::ToggleSelected(ArticleId(3)));
    assert_eq!(home.selected_articles(), Some(vec![ArticleId(2), ArticleId(4)]));
    let ticked: Vec<u64> = home.children()[4].children().iter().filter_map(|c| c.downcast_ref::<ArticleRow>()).filter(|r| r.is_selected()).map(|r| r.item.id.0).collect();
    assert_eq!(ticked, vec![4, 2], "the rows show it");
    sel(&mut home, &mut e, InoInkEntry::BulkApply(BulkAction::MarkRead));
    assert_eq!(e.context.inoink.refills, 1, "a bulk mark-read asks for a refill");
    {
        let st = e.context.inoink.store().unwrap();
        assert!(st.effective(ArticleId(2), &Membership::Read).unwrap());
        assert!(st.effective(ArticleId(4), &Membership::Read).unwrap());
        assert!(!st.effective(ArticleId(3), &Membership::Read).unwrap());
    }
    assert!(matches!(e.context.inoink.last_undo, Some(Undo::Bulk(_))));
    assert!(has_undo(&e));
    assert!(locate::<Notification>(&home).is_none());
    assert_eq!(home.children().len(), children);
    assert_eq!(rows(&home), vec![3, 1]);
    sel(&mut home, &mut e, InoInkEntry::UndoDone);
    assert!(!has_undo(&e));
    assert!(locate::<Notification>(&home).is_none());
    assert_eq!(home.children().len(), children);
    let st = e.context.inoink.store().unwrap();
    assert!(!st.effective(ArticleId(2), &Membership::Read).unwrap());
    assert!(!st.effective(ArticleId(4), &Membership::Read).unwrap());
    assert!(st.effective(ArticleId(2), &Membership::Starred).unwrap(), "the star is untouched");
}

#[test]
fn changing_tab_cancels_selection_and_the_row_menu_starts_one() {
    let mut e = env();
    let mut rq = RenderQueue::new();
    let mut home = InoInkHome::new(e.context.fb.rect(), &e.tx, &mut rq, &mut e.context);
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::Select(SelectPreset::None))));
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::Tab(Tab::ReadLater))));
    assert_eq!(home.selected_articles(), None);
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::Select(SelectPreset::One(ArticleId(2))))));
    assert_eq!(home.selected_articles(), Some(vec![ArticleId(2)]));
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
fn paging_past_the_last_page_shows_the_end_popup() {
    let mut e = env();
    let mut reader = open(&mut e, &[2, 1], true, 2);
    drive(&mut reader, &mut e, swipe_h(1000, 200));
    assert!(opens(&sent(&e)).is_empty());
    assert!(locate_by_id(&reader, ViewId::ArticleEnd).is_some());
    let st = e.context.inoink.store().unwrap();
    assert!(st.effective(ArticleId(2), &Membership::Starred).unwrap(), "still saved for later");
    assert!(!st.effective(ArticleId(2), &Membership::Read).unwrap(), "not marked read");
    assert!(locate_by_id(&reader, ViewId::QueueNav).is_none());
}

fn end_popup(reader: &Reader) -> &ArticleEnd {
    let i = locate_by_id(reader, ViewId::ArticleEnd).expect("end popup");
    reader.children()[i].downcast_ref::<ArticleEnd>().unwrap()
}

fn choose(reader: &mut Reader, e: &mut Env, c: EndChoice) {
    drive(reader, e, Event::Select(EntryId::InoInk(InoInkEntry::ArticleEnd(c))));
}

#[test]
fn end_popup_mark_read_keeps_the_star_and_opens_the_next() {
    let mut e = env();
    let mut reader = open(&mut e, &[2, 1], true, 2);
    drive(&mut reader, &mut e, swipe_h(1000, 200));
    assert_eq!(end_popup(&reader).choices(), vec![EndChoice::MarkRead, EndChoice::Unstar, EndChoice::KeepUnread]);
    choose(&mut reader, &mut e, EndChoice::MarkRead);
    assert_eq!(opens(&sent(&e)), vec![ArticleId(1)]);
    assert!(locate_by_id(&reader, ViewId::ArticleEnd).is_none());
    let st = e.context.inoink.store().unwrap();
    assert!(st.effective(ArticleId(2), &Membership::Read).unwrap());
    assert!(st.effective(ArticleId(2), &Membership::Starred).unwrap());
}

#[test]
fn end_popup_unstar_back_to_list() {
    let mut e = env();
    e.context.settings.inoink.after_end = AfterEnd::List;
    let mut reader = open(&mut e, &[2, 1], true, 2);
    drive(&mut reader, &mut e, swipe_h(1000, 200));
    choose(&mut reader, &mut e, EndChoice::Unstar);
    let evts = sent(&e);
    assert!(opens(&evts).is_empty());
    assert!(evts.iter().any(|ev| matches!(ev, Event::Back)));
    let st = e.context.inoink.store().unwrap();
    assert!(st.effective(ArticleId(2), &Membership::Read).unwrap());
    assert!(!st.effective(ArticleId(2), &Membership::Starred).unwrap());
    assert!(e.context.inoink.last_undo.is_some(), "undoable");
}

#[test]
fn end_popup_keep_unread_stays() {
    let mut e = env();
    let mut reader = open(&mut e, &[2, 1], true, 2);
    drive(&mut reader, &mut e, swipe_h(1000, 200));
    sent(&e);
    choose(&mut reader, &mut e, EndChoice::KeepUnread);
    assert!(locate_by_id(&reader, ViewId::ArticleEnd).is_none());
    assert!(sent(&e).iter().all(|ev| !matches!(ev, Event::OpenNext(_) | Event::Back)));
    assert!(!e.context.inoink.store().unwrap().effective(ArticleId(2), &Membership::Read).unwrap());
}

#[test]
fn end_popup_hides_unstar_when_not_starred() {
    let mut e = env();
    let mut reader = open(&mut e, &[4, 3], false, 4);
    drive(&mut reader, &mut e, swipe_h(1000, 200));
    assert_eq!(end_popup(&reader).choices(), vec![EndChoice::MarkRead, EndChoice::KeepUnread]);
}

#[test]
fn end_popup_toggle_is_remembered() {
    let mut e = env();
    let mut reader = open(&mut e, &[4, 3], false, 4);
    drive(&mut reader, &mut e, swipe_h(1000, 200));
    let r = end_popup(&reader).toggle_rects()[1];
    drive(&mut reader, &mut e, Event::Gesture(GestureEvent::Tap(Point::new((r.min.x + r.max.x) / 2, (r.min.y + r.max.y) / 2))));
    assert_eq!(e.context.settings.inoink.after_end, AfterEnd::List);
    assert_eq!(e.context.inoink.settings.after_end, AfterEnd::List);
    assert!(locate_by_id(&reader, ViewId::ArticleEnd).is_some(), "the popup stays open");
}

#[test]
fn forward_in_the_end_popup_skips_to_the_next() {
    let mut e = env();
    let mut reader = open(&mut e, &[4, 3], false, 4);
    drive(&mut reader, &mut e, swipe_h(1000, 200));
    sent(&e);
    drive(&mut reader, &mut e, press(ButtonCode::Forward, ButtonStatus::Pressed));
    drive(&mut reader, &mut e, press(ButtonCode::Forward, ButtonStatus::Released));
    assert_eq!(opens(&sent(&e)), vec![ArticleId(3)]);
    assert!(!e.context.inoink.store().unwrap().effective(ArticleId(4), &Membership::Read).unwrap());
}

#[test]
fn a_tap_outside_the_end_popup_closes_it() {
    let mut e = env();
    let mut reader = open(&mut e, &[4, 3], false, 4);
    drive(&mut reader, &mut e, swipe_h(1000, 200));
    drive(&mut reader, &mut e, Event::Gesture(GestureEvent::Tap(Point::new(20, 840))));
    assert!(locate_by_id(&reader, ViewId::ArticleEnd).is_none());
    assert!(opens(&sent(&e)).is_empty());
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
fn a_new_toast_replaces_the_old_one_below_the_top_bar() {
    let mut e = env();
    let mut reader = open(&mut e, &[4, 3], false, 4);
    drive(&mut reader, &mut e, swipe_v(600, 1400));
    drive(&mut reader, &mut e, swipe_v(600, 1400));
    let toasts: Vec<_> = reader.children().iter().filter(|c| c.is::<Notification>()).collect();
    assert_eq!(toasts.len(), 1, "one toast at a time");
    let r = *toasts[0].rect();
    assert_eq!(r.min.x, (1264 - r.width() as i32) / 2, "centred");
    assert!(r.min.y < 1680 / 4, "near the top");
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
fn tapping_the_title_in_the_bar_goes_back() {
    let mut e = env();
    let mut reader = open(&mut e, &[4, 3], false, 4);
    drive(&mut reader, &mut e, Event::Gesture(GestureEvent::Tap(plato_core::geom::Point::new(632, 840))));
    let index = locate::<ArticleBar>(&reader).expect("bar");
    let r = *reader.children()[index].rect();
    sent(&e);
    // Well right of the back arrow, on the title.
    let tap = Event::Gesture(GestureEvent::Tap(plato_core::geom::Point::new(r.min.x + 3 * r.height() as i32, (r.min.y + r.max.y) / 2)));
    let mut bus = VecDeque::new();
    plato_core::view::handle_event(&mut reader, &tap, &e.tx, &mut bus, &mut RenderQueue::new(), &mut e.context);
    // Like the back arrow, Back travels up to the app, which closes the reader.
    assert!(bus.iter().any(|ev| matches!(ev, Event::Back)));
}

#[test]
fn no_titles_row_without_neighbours() {
    let mut e = env();
    let mut reader = open(&mut e, &[4], false, 4);
    drive(&mut reader, &mut e, Event::Gesture(GestureEvent::Tap(plato_core::geom::Point::new(632, 840))));
    assert!(locate_by_id(&reader, ViewId::QueueNav).is_none());
}

#[test]
fn an_extracted_original_reopens_the_article_at_the_start() {
    let mut e = env();
    let mut reader = open(&mut e, &[4, 3], false, 4);
    sent(&e);
    let hash = e.context.inoink.store().unwrap().latest_version(ArticleId(4)).unwrap().unwrap().hash;
    let pos = ReadingPosition { content_hash: hash, offset: 7, anchor: String::new(), font_size: None };
    e.context.inoink.store_mut().unwrap().save_position(ArticleId(4), &pos).unwrap();
    drive(&mut reader, &mut e, Event::Sync(inoink::runtime::Update::Extracted(ArticleId(4), Ok(DownloadState::Complete))));
    let evts = sent(&e);
    assert!(evts.iter().any(|ev| matches!(ev, Event::ShowDocument(ArticleId(4), DocKind::Article))));
    assert!(!reader.children().iter().any(|c| c.is::<Notification>()), "no toast");
    assert!(e.context.inoink.open_at_start);
    let OpenResult::Ready(a) = open_article(&mut e.context, ArticleId(4)) else { panic!("cached") };
    assert_eq!(a.offset, None, "starts at the top of the new text");
    assert!(!e.context.inoink.open_at_start, "the flag is taken");
    let OpenResult::Ready(a) = open_article(&mut e.context, ArticleId(4)) else { panic!("cached") };
    assert_eq!(a.offset, Some(7), "later opens resume");
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
fn the_title_stays_in_the_top_margin_after_the_first_page() {
    let mut e = env();
    add_long_article(&mut e, 9);
    e.context.inoink.open_at_end = true;
    let mut reader = open(&mut e, &[9], false, 9);
    assert!(!has_bar(&reader));
    assert_eq!(reader.running_header(), Some("Article 9"));
    drive(&mut reader, &mut e, Event::Gesture(GestureEvent::Tap(plato_core::geom::Point::new(100, 840))));
    assert_eq!(reader.running_header(), Some("Article 9"));
}

#[test]
fn an_article_opened_at_its_start_shows_no_bar() {
    let mut e = env();
    let reader = open(&mut e, &[4, 3], false, 4);
    assert!(!has_bar(&reader));
    assert_eq!(reader.running_header(), None, "the first page has the article's own heading");
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

fn has_dialog(home: &InoInkHome) -> bool {
    locate_by_id(home, ViewId::ConfirmDialog).is_some()
}

#[test]
fn mark_all_read_asks_first_and_skips_saved() {
    let mut e = env();
    let mut rq = RenderQueue::new();
    let mut home = InoInkHome::new(e.context.fb.rect(), &e.tx, &mut rq, &mut e.context);
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::MarkRead(ListSpan::All))));
    assert!(has_dialog(&home), "it asks first");
    assert!(!e.context.inoink.store().unwrap().effective(ArticleId(3), &Membership::Read).unwrap());
    assert_eq!(home.selected_articles(), None, "no selection mode");
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::ConfirmMarkRead(ListSpan::All))));
    let st = e.context.inoink.store().unwrap();
    assert!(st.effective(ArticleId(3), &Membership::Read).unwrap());
    assert!(st.effective(ArticleId(4), &Membership::Read).unwrap());
    assert!(!st.effective(ArticleId(1), &Membership::Read).unwrap(), "saved stays unread");
    assert!(!st.effective(ArticleId(2), &Membership::Read).unwrap());
    assert!(matches!(e.context.inoink.last_undo, Some(Undo::Bulk(_))));
    assert!(locate::<Notification>(&home).is_none());
    assert_eq!(e.context.inoink.refills, 1);
}

#[test]
fn mark_read_up_to_and_from_here_follow_the_list() {
    let mut e = env();
    e.context.inoink.store_mut().unwrap().upsert_article(&rec(5, 500_000_000, false), 0).unwrap();
    let mut rq = RenderQueue::new();
    let mut home = InoInkHome::new(e.context.fb.rect(), &e.tx, &mut rq, &mut e.context);
    assert_eq!(rows(&home), vec![5, 4, 3, 2, 1]);
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::ConfirmMarkRead(ListSpan::UpTo(ArticleId(4))))));
    {
        let st = e.context.inoink.store().unwrap();
        assert!(st.effective(ArticleId(5), &Membership::Read).unwrap());
        assert!(st.effective(ArticleId(4), &Membership::Read).unwrap());
        assert!(!st.effective(ArticleId(3), &Membership::Read).unwrap());
    }
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::UndoDone)));
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::ConfirmMarkRead(ListSpan::From(ArticleId(4))))));
    let st = e.context.inoink.store().unwrap();
    assert!(st.effective(ArticleId(4), &Membership::Read).unwrap());
    assert!(st.effective(ArticleId(3), &Membership::Read).unwrap());
    assert!(!st.effective(ArticleId(5), &Membership::Read).unwrap());
    assert!(!st.effective(ArticleId(1), &Membership::Read).unwrap());
    assert!(!st.effective(ArticleId(2), &Membership::Read).unwrap());
}

#[test]
fn the_end_card_marks_in_one_tap() {
    let mut e = env();
    let mut rq = RenderQueue::new();
    let mut home = InoInkHome::new(e.context.fb.rect(), &e.tx, &mut rq, &mut e.context);
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::MarkSeenRead)));
    assert!(!has_dialog(&home));
    assert_eq!(home.selected_articles(), None);
    let st = e.context.inoink.store().unwrap();
    assert!(st.effective(ArticleId(3), &Membership::Read).unwrap());
    assert!(st.effective(ArticleId(4), &Membership::Read).unwrap());
    assert!(!st.effective(ArticleId(1), &Membership::Read).unwrap());
    assert!(!st.effective(ArticleId(2), &Membership::Read).unwrap());
    assert_eq!(e.context.inoink.refills, 1);
}

#[test]
fn row_menu_offers_mark_read_up_to_here_except_in_read_later() {
    let mut e = env();
    let mut rq = RenderQueue::new();
    let mut home = InoInkHome::new(e.context.fb.rect(), &e.tx, &mut rq, &mut e.context);
    let item = e.context.inoink.store().unwrap().query_articles(&ArticleQuery { collection: Some(Collection::All), limit: 1, ..Default::default() }).unwrap().items[0].clone();
    let labels = |home: &InoInkHome, e: &Env| -> Vec<String> {
        home.row_menu_entries(&item, &e.context).into_iter().filter_map(|k| if let EntryKind::Command(l, _) = k { Some(l) } else { None }).collect()
    };
    let l = labels(&home, &e);
    assert!(l.contains(&"Mark read up to here".to_string()) && l.contains(&"Mark read from here down".to_string()), "{l:?}");
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::Tab(Tab::ReadLater))));
    let l = labels(&home, &e);
    assert!(!l.contains(&"Mark read up to here".to_string()) && !l.contains(&"Mark read from here down".to_string()), "{l:?}");
}

#[test]
fn comments_button_toggles_documents() {
    let mut e = env();
    let rect = Rectangle::new(Point::new(100, 20), Point::new(280, 140));
    for current in [DocKind::Article, DocKind::Comments] {
        let other = if current == DocKind::Article { DocKind::Comments } else { DocKind::Article };
        let mut button = CommentsButton::new(rect, ArticleId(42), current, Some(3));
        for y in [20, 80, 139] {
            let mut bus = VecDeque::new();
            let mut rq = RenderQueue::new();
            assert!(button.handle_event(&Event::Gesture(Gest::Tap(Point::new(150, y))), &e.tx, &mut bus, &mut rq, &mut e.context));
            assert!(matches!(bus.pop_front(), Some(Event::ShowDocument(ArticleId(42), k)) if k == other));
            assert!(bus.is_empty());
        }
        let mut bus = VecDeque::new();
        let mut rq = RenderQueue::new();
        assert!(button.handle_event(&Event::Gesture(Gest::HoldFingerShort(Point::new(150, 80), 0)), &e.tx, &mut bus, &mut rq, &mut e.context));
        assert!(!button.handle_event(&Event::Gesture(Gest::Tap(Point::new(99, 80))), &e.tx, &mut bus, &mut rq, &mut e.context));
        assert!(bus.is_empty());
    }
}

#[test]
fn comments_button_shows_count_and_article_return() {
    let mut e = env();
    let rect = Rectangle::new(Point::new(50, 20), Point::new(230, 140));
    let render = |e: &mut Env, current: DocKind, count: Option<u32>, image: &mut Pixmap| {
        CommentsButton::new(rect, ArticleId(42), current, count).render(image, rect, &mut e.context.fonts);
    };
    let dark = |image: &Pixmap| (0..160).flat_map(|y| (0..300).map(move |x| (x, y))).filter(|&(x, y)| image.get_pixel(x, y).gray() < 128).count();
    let mut outline = Pixmap::new(300, 160, 3);
    render(&mut e, DocKind::Article, Some(42), &mut outline);
    let mut back = Pixmap::new(300, 160, 3);
    render(&mut e, DocKind::Comments, Some(42), &mut back);
    let mut back_no_count = Pixmap::new(300, 160, 3);
    render(&mut e, DocKind::Comments, None, &mut back_no_count);
    let mut bare = Pixmap::new(300, 160, 3);
    render(&mut e, DocKind::Article, None, &mut bare);
    for img in [&outline, &back, &bare] {
        for y in 0..160 {
            for x in 0..300 {
                if !rect.includes(Point::new(x as i32, y as i32)) {
                    assert_eq!(img.get_pixel(x, y).gray(), 255, "write outside at {x},{y}");
                }
            }
        }
    }
    assert!(dark(&outline) > dark(&bare), "the count adds ink");
    assert!(dark(&bare) > 0 && dark(&back) > 0);
    assert_eq!(back.data, back_no_count.data, "the article button shows no count");
    assert_ne!(back.data, bare.data, "a different icon on the comments");
    render(&mut e, DocKind::Comments, Some(42), &mut outline);
    assert_eq!(outline.data, back.data, "switching clears the old state");
}

#[test]
fn article_bar_layout_with_comments() {
    let mut e = env();
    for width in [1264, 1680, 758] {
        let bar_rect = Rectangle::new(Point::new(0, 10), Point::new(width, 131));
        let side = bar_rect.height() as i32;
        for count in [None, Some(0), Some(1), Some(42), Some(12345)] {
            for current in [DocKind::Article, DocKind::Comments] {
                let mut bar = ArticleBar::new(bar_rect, ArticleId(42), "Title".into(), false, Some((current, count)), &mut e.context);
                let button = bar.children().last().unwrap().downcast_ref::<CommentsButton>().expect("button is last");
                let aa = bar.children()[bar.children().len() - 2].rect();
                assert_eq!(button.rect().max.x, aa.min.x);
                assert!(button.rect().min.x >= side);
                assert_eq!(bar.children()[1].rect().max.x, button.rect().min.x);
                assert!(bar.children()[2].downcast_ref::<plato_core::view::icon::Icon>().is_some_and(|i| i.name == "bookmark"));
                let mut rq = RenderQueue::new();
                bar.set_starred(true, &mut rq);
                assert!(bar.children()[2].downcast_ref::<plato_core::view::icon::Icon>().is_some_and(|i| i.name == "bookmark-filled"));
            }
        }
    }
}

#[test]
fn headline_tap_opens_ribbon_tap_saves_hold_shows_the_menu() {
    let e = env();
    let item = e.context.inoink.store().unwrap().query_articles(&ArticleQuery { collection: Some(Collection::All), limit: 1, ..Default::default() }).unwrap().items[0].clone();
    let id = item.id;
    let rect = Rectangle::new(Point::new(0, 0), Point::new(1264, 163));
    let mut row = ArticleRow::new(rect, item, 0, RowMode::OpenMenu, false);
    let mut e = e;
    let mut run = |g: GestureEvent| {
        let mut bus = VecDeque::new();
        let mut rq = RenderQueue::new();
        row.handle_event(&Event::Gesture(g), &e.tx, &mut bus, &mut rq, &mut e.context);
        bus
    };
    let bus = run(Gest::Tap(Point::new(rect.max.x - 5, 80)));
    assert!(matches!(bus.front(), Some(Event::ArticleAction(i, ArticleAction::ToggleStar)) if *i == id));
    let bus = run(Gest::Tap(Point::new(400, 80)));
    assert!(matches!(bus.front(), Some(Event::OpenArticle(i)) if *i == id));
    let bus = run(Gest::HoldFingerShort(Point::new(400, 80), 0));
    assert!(matches!(bus.front(), Some(Event::ToggleBookMenu(..))));
}

#[test]
fn the_select_button_enters_and_leaves_selection_mode() {
    let mut e = env();
    let mut rq = RenderQueue::new();
    let mut home = InoInkHome::new(e.context.fb.rect(), &e.tx, &mut rq, &mut e.context);
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::ToggleSelectMode)));
    assert_eq!(home.selected_articles(), Some(vec![]), "nothing ticked");
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::ToggleSelectMode)));
    assert_eq!(home.selected_articles(), None);
}

#[test]
fn archive_is_silent_and_menu_undo_survives_paging() {
    let mut e = env();
    for i in 10..30u64 {
        e.context.inoink.store_mut().unwrap().upsert_article(&rec(i, (i as i64) * 1_000_000_000, false), 0).unwrap();
    }
    let mut rq = RenderQueue::new();
    let mut home = InoInkHome::new(e.context.fb.rect(), &e.tx, &mut rq, &mut e.context);
    let before = rows(&home).len();
    let children = home.children().len();
    send(&mut home, &mut e, Event::ToggleBookMenu(plato_core::geom::Rectangle::default(), 0));
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::Article(ArticleAction::Done))));
    assert!(matches!(e.context.inoink.last_undo, Some(Undo::Done(_))));
    assert!(locate::<Notification>(&home).is_none());
    assert_eq!(home.children().len(), children);
    assert_eq!(rows(&home).len(), before, "full row capacity remains");
    let last_on_first = *rows(&home).last().unwrap();
    send(&mut home, &mut e, Event::Page(CycleDir::Next));
    assert!(has_undo(&e));
    assert_eq!(rows(&home)[0], last_on_first - 1, "the next page starts after the last headline shown");
    send(&mut home, &mut e, Event::Reseed);
    assert!(has_undo(&e));
    assert!(locate::<Notification>(&home).is_none());
}

#[test]
fn menu_undo_restores_silently_and_disappears() {
    let mut e = env();
    let mut rq = RenderQueue::new();
    let mut home = InoInkHome::new(e.context.fb.rect(), &e.tx, &mut rq, &mut e.context);
    assert!(!has_undo(&e));
    send(&mut home, &mut e, Event::ToggleBookMenu(plato_core::geom::Rectangle::default(), 0));
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::Article(ArticleAction::Done))));
    assert!(has_undo(&e));
    assert!(e.context.inoink.store().unwrap().effective(ArticleId(4), &Membership::Read).unwrap());
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::UndoDone)));
    assert!(locate::<Notification>(&home).is_none());
    assert!(!e.context.inoink.store().unwrap().effective(ArticleId(4), &Membership::Read).unwrap());
    assert_eq!(rows(&home), vec![4, 3, 2, 1]);
    assert!(e.context.inoink.last_undo.is_none());
    assert!(!has_undo(&e));
    send(&mut home, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::UndoDone)));
    assert!(locate::<Notification>(&home).is_none());
    assert!(!has_undo(&e));
}

#[test]
fn latest_successful_action_replaces_menu_undo() {
    let mut e = env();
    let outcome = apply_article_action(&mut e.context, ArticleId(4), &ArticleAction::Done).unwrap();
    assert!(outcome.message.is_none());
    let first = e.context.inoink.last_undo;
    assert!(matches!(first, Some(Undo::Done(_))));
    let store = e.context.inoink.store.take();
    assert_eq!(apply_bulk_action(&mut e.context, &[ArticleId(3)], BulkAction::MarkRead).unwrap_err(), SAVE_FAILED);
    assert_eq!(e.context.inoink.last_undo, first);
    assert_eq!(undo_done(&mut e.context).unwrap_err(), SAVE_FAILED);
    assert_eq!(e.context.inoink.last_undo, first);
    e.context.inoink.store = store;
    assert_eq!(apply_bulk_action(&mut e.context, &[ArticleId(3)], BulkAction::MarkRead).unwrap(), 1);
    assert!(matches!(e.context.inoink.last_undo, Some(Undo::Bulk(_))));
    assert!(has_undo(&e));
    assert!(undo_done(&mut e.context).unwrap().message.is_none());
    let st = e.context.inoink.store().unwrap();
    assert!(st.effective(ArticleId(4), &Membership::Read).unwrap());
    assert!(!st.effective(ArticleId(3), &Membership::Read).unwrap());
    assert!(e.context.inoink.last_undo.is_none());
    assert!(!has_undo(&e));
}
