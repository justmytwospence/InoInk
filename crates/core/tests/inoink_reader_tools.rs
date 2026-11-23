//! Reader tools: the image viewer, Return after a jump, highlight styles and the highlight
//! export. Run through scripts/container-run.sh (links MuPDF).

use inoink::cache::{Cache, CacheConfig, Control};
use inoink::content::images::{Fetched, Fetcher};
use inoink::model::*;
use inoink::store::{ArticleRecord, Store};
use plato_core::battery::FakeBattery;
use plato_core::context::Context;
use plato_core::font::Fonts;
use plato_core::framebuffer::{Framebuffer, Pixmap};
use plato_core::frontlight::LightLevels;
use plato_core::geom::{Dir, Point, Rectangle};
use plato_core::gesture::GestureEvent;
use plato_core::settings::Settings;
use plato_core::view::button::Button;
use plato_core::view::inoink::image_view::{ImageView, ZoomStep};
use plato_core::view::inoink::state::InoInk;
use plato_core::view::{Event, RenderQueue, View};
use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc};

struct NoNet;
impl Fetcher for NoNet {
    fn get(&self, _u: &url::Url, _m: usize) -> inoink::Result<Fetched> {
        Ok(Fetched { status: 404, ..Default::default() })
    }
}

fn rec(id: u64, summary: &str) -> ArticleRecord {
    ArticleRecord {
        id,
        title: format!("Article {id}"),
        origin_stream: "feed/https://x.example/rss".into(),
        origin_title: "X".into(),
        canonical_url: format!("https://x.example/{id}"),
        timestamp_usec: id as i64 * 1_000_000,
        published: id as i64,
        summary_html: summary.to_string(),
        raw_json: "{}".into(),
        categories: vec!["user/1/state/com.google/reading-list".to_string()],
        ..Default::default()
    }
}

struct Env {
    _dir: tempfile::TempDir,
    context: Context,
    tx: mpsc::Sender<Event>,
    rx: mpsc::Receiver<Event>,
}

fn env_with(articles: &[(u64, &str)]) -> Env {
    std::env::set_current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    {
        let mut store = Store::open(&data.join("inoink.db"), "local").unwrap();
        let cache = Cache::new(&data.join("cache"), "local", CacheConfig::default());
        let ctl = Control::new(Arc::new(AtomicBool::new(false)));
        for (id, summary) in articles {
            store.upsert_article(&rec(*id, summary), 0).unwrap();
            cache.cache_article(&mut store, &NoNet, ArticleId(*id), &ctl).unwrap();
        }
        store.checkpoint().unwrap();
    }
    let mut settings = Settings::default();
    settings.inoink.data_dir = data;
    let (tx, rx) = mpsc::channel();
    let inoink = InoInk::open(&settings.inoink, tx.clone(), false).unwrap();
    let fb = Box::new(Pixmap::new(1264, 1680, 3));
    let context = Context::new(fb, None, inoink, settings, Fonts::load().unwrap(), Box::new(FakeBattery::new()), Box::new(LightLevels::default()), Box::new(0u16));
    Env { _dir: dir, context, tx, rx }
}

fn drive(view: &mut dyn View, e: &mut Env, evt: Event) {
    let mut bus = VecDeque::new();
    let mut rq = RenderQueue::new();
    plato_core::view::handle_event(view, &evt, &e.tx, &mut bus, &mut rq, &mut e.context);
    while let Some(b) = bus.pop_front() {
        plato_core::view::handle_event(view, &b, &e.tx, &mut VecDeque::new(), &mut rq, &mut e.context);
    }
}

fn sent(e: &Env) -> Vec<Event> {
    let mut out = Vec::new();
    while let Ok(evt) = e.rx.try_recv() {
        out.push(evt);
    }
    out
}

fn tap(x: i32, y: i32) -> Event {
    Event::Gesture(GestureEvent::Tap(Point::new(x, y)))
}

fn tap_rect(r: Rectangle) -> Event {
    let c = r.center();
    tap(c.x, c.y)
}

// ---- image viewer --------------------------------------------------------------------------

fn image(e: &Env) -> std::path::PathBuf {
    let path = e.context.inoink.data_dir.join("picture.png");
    let mut img = Pixmap::new(400, 300, 3);
    img.draw_rectangle(&plato_core::geom::Rectangle::new(Point::new(0, 0), Point::new(200, 150)), plato_core::color::BLACK);
    img.save(path.to_str().unwrap()).unwrap();
    path
}

fn buttons(v: &ImageView) -> Vec<(String, Rectangle, bool)> {
    // Button keeps its label private; the order is Zoom out, Fit, Zoom in.
    v.children().iter().filter_map(|c| c.downcast_ref::<Button>()).zip(["Zoom out", "Fit", "Zoom in"]).map(|(b, l)| (l.to_string(), *b.rect(), b.disabled)).collect()
}

#[test]
fn image_viewer_fits_then_zooms_with_pinches_and_buttons() {
    let mut e = env_with(&[]);
    let path = image(&e);
    let rect = e.context.fb.rect();
    let mut v = ImageView::new(rect, path, &mut e.context);
    // 400x300 fits 1264 wide: scale 3.16.
    assert_eq!(v.zoom(), 1.0);
    assert_eq!(v.rendered_size(), Some((1264, 948)));
    assert!(buttons(&v)[0].2 && buttons(&v)[1].2, "Zoom out and Fit are off at fit");
    assert!(!buttons(&v)[2].2, "Zoom in is on");

    drive(&mut v, &mut e, Event::Gesture(GestureEvent::Spread { axis: plato_core::geom::Axis::Diagonal, center: rect.center(), factor: 2.0 }));
    assert!((v.zoom() - 2.0).abs() < 0.01);
    assert_eq!(v.rendered_size(), Some((2528, 1896)));
    // The image centre stays at the screen centre.
    assert_eq!(v.offset(), Point::new(632, 108));

    drive(&mut v, &mut e, Event::Gesture(GestureEvent::Pinch { axis: plato_core::geom::Axis::Diagonal, center: rect.center(), factor: 0.75 }));
    assert!((v.zoom() - 1.5).abs() < 0.01);

    // The buttons step the zoom up to the limit, and Fit goes back.
    let zoom_in = buttons(&v)[2].1;
    for _ in 0..6 {
        drive(&mut v, &mut e, tap_rect(zoom_in));
    }
    assert_eq!(v.zoom(), v.max_zoom());
    assert!(v.max_zoom() <= plato_core::view::inoink::image_view::MAX_ZOOM);
    assert!(buttons(&v)[2].2, "Zoom in is off at the limit");
    let fit = buttons(&v)[1].1;
    drive(&mut v, &mut e, tap_rect(fit));
    assert_eq!(v.zoom(), 1.0);
    drive(&mut v, &mut e, Event::ImageZoom(ZoomStep::In));
    assert!((v.zoom() - 1.5).abs() < 0.01);
    drive(&mut v, &mut e, Event::ImageZoom(ZoomStep::Out));
    assert_eq!(v.zoom(), 1.0);
}

#[test]
fn image_viewer_pans_by_dragging_and_double_tap_toggles() {
    let mut e = env_with(&[]);
    let path = image(&e);
    let rect = e.context.fb.rect();
    let mut v = ImageView::new(rect, path, &mut e.context);

    // Dragging an image that fits does nothing.
    drive(&mut v, &mut e, Event::Gesture(GestureEvent::Swipe { dir: Dir::West, start: Point::new(800, 800), end: Point::new(500, 800) }));
    assert_eq!(v.offset(), Point::new(0, 0));

    // Double tap enlarges around the tap...
    drive(&mut v, &mut e, tap(632, 840));
    assert_eq!(v.zoom(), 1.0, "a single tap does nothing");
    drive(&mut v, &mut e, tap(634, 842));
    assert!((v.zoom() - 2.0).abs() < 0.01);
    let at = v.offset();

    // ...the content follows the finger...
    drive(&mut v, &mut e, Event::Gesture(GestureEvent::Swipe { dir: Dir::North, start: Point::new(600, 900), end: Point::new(500, 850) }));
    assert_eq!(v.offset(), Point::new(at.x + 100, at.y + 50));
    // ...stops at the edges...
    drive(&mut v, &mut e, Event::Gesture(GestureEvent::Swipe { dir: Dir::East, start: Point::new(100, 900), end: Point::new(1200, 900) }));
    assert_eq!(v.offset().x, 0);

    // ...and a second double tap fits again.
    drive(&mut v, &mut e, tap(400, 400));
    drive(&mut v, &mut e, tap(400, 400));
    assert_eq!(v.zoom(), 1.0);
    assert_eq!(v.offset(), Point::new(0, 0));
}

#[test]
fn image_viewer_remembers_each_images_zoom() {
    let mut e = env_with(&[]);
    let path = image(&e);
    let rect = e.context.fb.rect();
    let mut v = ImageView::new(rect, path.clone(), &mut e.context);
    drive(&mut v, &mut e, Event::Gesture(GestureEvent::Spread { axis: plato_core::geom::Axis::Diagonal, center: rect.center(), factor: 2.0 }));
    drive(&mut v, &mut e, Event::Gesture(GestureEvent::Swipe { dir: Dir::North, start: Point::new(600, 900), end: Point::new(500, 700) }));
    let (zoom, offset) = (v.zoom(), v.offset());
    drop(v);

    let again = ImageView::new(rect, path.clone(), &mut e.context);
    assert!((again.zoom() - zoom).abs() < 0.01);
    assert!((again.offset().x - offset.x).abs() <= 2 && (again.offset().y - offset.y).abs() <= 2, "{:?} vs {:?}", again.offset(), offset);

    // Another image starts fitted.
    let other = e.context.inoink.data_dir.join("other.png");
    std::fs::copy(&path, &other).unwrap();
    assert_eq!(ImageView::new(rect, other, &mut e.context).zoom(), 1.0);
    let _ = sent(&e);
}

// ---- Return after a jump -------------------------------------------------------------------

use plato_core::document::Location;
use plato_core::view::inoink::actions::{open_article, DocKind, InoInkEntry, OpenResult};
use plato_core::view::reader::Reader;
use plato_core::view::EntryId;

fn long_article() -> String {
    let mut s = String::from("<h2>Intro</h2><p>See the note<a href=\"#fn1\">1</a>.</p>");
    for i in 0..80 {
        s += &format!("<p>Paragraph {i} has enough words in it to fill the page and push the note far away from where it is cited.</p>");
    }
    s += "<h2>Notes</h2><h3>First</h3><p id=\"fn1\">The note.</p>";
    s
}

fn open_reader(e: &mut Env, id: u64) -> Reader {
    let OpenResult::Ready(article) = open_article(&mut e.context, ArticleId(id)) else { panic!("cached article should open") };
    let mut reader = Reader::new_article(e.context.fb.rect(), &article, &e.tx, &mut e.context).expect("reader");
    drive(&mut reader, e, Event::Update(plato_core::framebuffer::UpdateMode::Partial));
    reader
}

fn page(r: &Reader) -> usize {
    r.page_position().expect("article page").0
}

#[test]
fn a_footnote_jump_offers_return_to_the_page() {
    let long = long_article();
    let mut e = env_with(&[(1, &long)]);
    let mut reader = open_reader(&mut e, 1);
    assert_eq!(page(&reader), 0);
    assert!(reader.return_label().is_none());
    assert!(reader.page_position().unwrap().1 > 3, "the note is pages away");

    // Following the note's link (a local URI) jumps and offers the way back.
    drive(&mut reader, &mut e, Event::GoToLocation(Location::LocalUri(0, "#fn1".into())));
    assert_eq!(page(&reader), reader.page_position().unwrap().1 - 1);
    assert_eq!(reader.return_label(), Some("Return to page 1"));

    let pill = reader.return_rect().expect("Return is shown");
    assert!(pill.max.y < 120, "in the top margin: {pill:?}");
    drive(&mut reader, &mut e, tap_rect(pill));
    assert_eq!(page(&reader), 0);
    assert!(reader.return_label().is_none() && reader.return_rect().is_none());
}

#[test]
fn return_goes_away_when_held_or_when_paging_back() {
    let long = long_article();
    let mut e = env_with(&[(1, &long)]);
    let mut reader = open_reader(&mut e, 1);
    drive(&mut reader, &mut e, Event::GoToLocation(Location::LocalUri(0, "#fn1".into())));
    let pill = reader.return_rect().unwrap();
    drive(&mut reader, &mut e, Event::Gesture(GestureEvent::HoldFingerShort(pill.center(), 0)));
    assert!(reader.return_label().is_none());
    let last = page(&reader);
    assert!(last > 0, "holding stays");

    // Jump again, then page back by hand to where the jump started.
    drive(&mut reader, &mut e, Event::GoToLocation(Location::Exact(0)));
    assert_eq!(page(&reader), 0);
    assert_eq!(reader.return_label().map(String::from), Some(format!("Return to page {}", last + 1)));
    for _ in 0..last {
        drive(&mut reader, &mut e, tap(1200, 900));
    }
    assert_eq!(page(&reader), last);
    assert!(reader.return_label().is_none(), "back on the page the jump left");
}

#[test]
fn contents_lists_headings_and_returns_after_a_jump() {
    let long = long_article();
    let mut e = env_with(&[(1, &long)]);
    let mut reader = open_reader(&mut e, 1);
    sent(&e);
    drive(&mut reader, &mut e, Event::Select(EntryId::InoInk(InoInkEntry::Contents)));
    let html = sent(&e).into_iter().find_map(|ev| if let Event::OpenHtml(html, _) = ev { Some(html) } else { None }).expect("the contents open");
    for heading in ["Article 1", "Intro", "Notes", "First"] {
        assert!(html.contains(heading), "{heading} in {html}");
    }
    // Choosing "First" (the last entry) jumps there the way the contents view does.
    let href = html.rsplit("href=\"@").next().unwrap();
    let offset: usize = href[..href.find('"').unwrap()].parse().unwrap();
    drive(&mut reader, &mut e, Event::GoToLocation(Location::Exact(offset)));
    assert!(page(&reader) > 0);
    assert_eq!(reader.return_label(), Some("Return to page 1"));
}

#[test]
fn switching_to_the_other_document_can_return() {
    let long = long_article();
    let mut e = env_with(&[(1, &long), (2, "<p>Short.</p>")]);
    let mut reader = open_reader(&mut e, 1);
    // The article bar's switch passes through the reader on its way to the app.
    drive(&mut reader, &mut e, Event::ShowDocument(ArticleId(1), DocKind::Comments));
    assert_eq!(e.context.inoink.doc_return, Some((ArticleId(1), DocKind::Article)));

    // The comments would open next; here the article stands in, opened as if from them.
    e.context.inoink.doc_return = Some((ArticleId(1), DocKind::Comments));
    let mut reader = open_reader(&mut e, 1);
    assert!(e.context.inoink.doc_return.is_none(), "taken by the new reader");
    assert_eq!(reader.return_label(), Some("Return to the comments"));
    sent(&e);
    let pill = reader.return_rect().unwrap();
    drive(&mut reader, &mut e, tap_rect(pill));
    assert!(sent(&e).iter().any(|ev| matches!(ev, Event::ShowDocument(ArticleId(1), DocKind::Comments))));
    assert!(e.context.inoink.doc_return.is_none(), "going back is not a new jump");

    // A pending return belongs to its article only.
    e.context.inoink.doc_return = Some((ArticleId(1), DocKind::Comments));
    let other = open_reader(&mut e, 2);
    assert!(other.return_label().is_none());
    assert!(e.context.inoink.doc_return.is_none());
}

fn linked_article(href: &str, target: &str) -> String {
    let mut s = format!("<p><a href=\"#{href}\">Follow this link to the far end of the text</a></p>");
    for i in 0..80 {
        s += &format!("<p>Paragraph {i} has enough words in it to fill the page and push the target far away.</p>");
    }
    s += &format!("<p id=\"{target}\">The far end.</p>");
    s
}

/// Taps down the left side of the first page until a link is followed (a miss pages back, which
/// does nothing on the first page).
fn tap_first_link(reader: &mut Reader, e: &mut Env) {
    for y in (250..700).step_by(8) {
        drive(reader, e, tap(200, y));
        if page(reader) > 0 {
            return;
        }
        for evt in sent(e) {
            if let Event::GoTo(_) = evt {
                drive(reader, e, evt);
                return;
            }
        }
    }
    panic!("no link found");
}

#[test]
fn tapping_a_link_offers_return_but_skipping_a_thread_does_not() {
    let a = linked_article("note-1", "note-1");
    let b = linked_article("inoink-t2", "inoink-t2");
    let mut e = env_with(&[(1, &a), (2, &b)]);

    let mut reader = open_reader(&mut e, 1);
    tap_first_link(&mut reader, &mut e);
    assert!(page(&reader) > 0);
    assert_eq!(reader.return_label(), Some("Return to page 1"));

    let mut reader = open_reader(&mut e, 2);
    tap_first_link(&mut reader, &mut e);
    assert!(page(&reader) > 0);
    assert!(reader.return_label().is_none(), "Skip reads on");
}
