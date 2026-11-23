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
