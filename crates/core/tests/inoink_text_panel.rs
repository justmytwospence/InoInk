//! The Aa panel: live reflow while a slider is dragged (throttled, anchored to the first word of
//! the page, saved on release), font faces in the picker and text presets. Run through
//! scripts/container-run.sh (links MuPDF).

use inoink::cache::{Cache, CacheConfig, Control};
use inoink::content::images::{Fetched, Fetcher};
use inoink::model::*;
use inoink::store::{ArticleRecord, Store};
use plato_core::battery::FakeBattery;
use plato_core::context::Context;
use plato_core::device::CURRENT_DEVICE;
use plato_core::font::Fonts;
use plato_core::framebuffer::Pixmap;
use plato_core::frontlight::LightLevels;
use plato_core::geom::{CycleDir, Point, Rectangle};
use plato_core::gesture::GestureEvent;
use plato_core::input::{DeviceEvent, FingerStatus};
use plato_core::metadata::TextAlign;
use plato_core::settings::{Settings, DEFAULT_FONT_SIZE};
use plato_core::view::common::locate_by_id;
use plato_core::view::inoink::actions::{open_article, OpenResult};
use plato_core::view::inoink::state::InoInk;
use plato_core::view::inoink::text_panel::{PresetAction, TextPanel};
use plato_core::view::reader::Reader;
use plato_core::view::{EntryId, Event, RenderQueue, SliderId, View, ViewId};
use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc};

struct NoNet;
impl Fetcher for NoNet {
    fn get(&self, _u: &url::Url, _m: usize) -> inoink::Result<Fetched> {
        Ok(Fetched { status: 404, ..Default::default() })
    }
}

struct Env {
    _dir: tempfile::TempDir,
    context: Context,
    tx: mpsc::Sender<Event>,
    rx: mpsc::Receiver<Event>,
}

fn long_article() -> String {
    (0..60).map(|i| format!("<p>Paragraph {i} has enough words in it to fill the page several times over, so that every change of the text moves the page breaks.</p>")).collect()
}

fn env() -> Env {
    std::env::set_current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    {
        let mut store = Store::open(&data.join("inoink.db"), "local").unwrap();
        let cache = Cache::new(&data.join("cache"), "local", CacheConfig::default());
        let ctl = Control::new(Arc::new(AtomicBool::new(false)));
        let rec = ArticleRecord {
            id: 1,
            title: "Article 1".into(),
            origin_stream: "feed/https://x.example/rss".into(),
            origin_title: "X".into(),
            canonical_url: "https://x.example/1".into(),
            timestamp_usec: 1_000_000,
            published: 1,
            summary_html: long_article(),
            raw_json: "{}".into(),
            categories: vec!["user/1/state/com.google/reading-list".to_string()],
            ..Default::default()
        };
        store.upsert_article(&rec, 0).unwrap();
        cache.cache_article(&mut store, &NoNet, ArticleId(1), &ctl).unwrap();
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

/// Events sent to the hub since the last call, without the page prefetching's.
fn sent(e: &Env) -> Vec<Event> {
    let mut out = Vec::new();
    while let Ok(evt) = e.rx.try_recv() {
        if !matches!(evt, Event::LoadPixmap(..)) {
            out.push(evt);
        }
    }
    out
}

fn ticks(e: &Env) -> usize {
    sent(e).iter().filter(|evt| matches!(evt, Event::LiveTextTick)).count()
}

fn tap_rect(r: Rectangle) -> Event {
    Event::Gesture(GestureEvent::Tap(r.center()))
}

fn finger(status: FingerStatus, position: Point) -> Event {
    Event::Device(DeviceEvent::Finger { id: 0, time: 0.0, status, position })
}

/// Opens the article on its third page with the Aa panel up.
fn open_panel(e: &mut Env) -> Reader {
    let OpenResult::Ready(article) = open_article(&mut e.context, ArticleId(1)) else { panic!("cached article should open") };
    let mut reader = Reader::new_article(e.context.fb.rect(), &article, &e.tx, &mut e.context).expect("reader");
    drive(&mut reader, e, Event::Update(plato_core::framebuffer::UpdateMode::Partial));
    drive(&mut reader, e, Event::Page(CycleDir::Next));
    drive(&mut reader, e, Event::Page(CycleDir::Next));
    assert_eq!(reader.page_position().unwrap().0, 2);
    drive(&mut reader, e, Event::Toggle(ViewId::TextPanel));
    assert!(locate_by_id(&reader, ViewId::TextPanel).is_some());
    sent(e);
    reader
}

fn panel(reader: &Reader) -> &TextPanel {
    let i = locate_by_id(reader, ViewId::TextPanel).expect("text panel");
    reader.children()[i].downcast_ref::<TextPanel>().unwrap()
}

/// The point of a slider's track for a value (see `Slider::update_value`).
fn slider_x(track: Rectangle, value: f32, (min, max): (f32, f32)) -> Point {
    // `scale_by_dpi(BUTTON_DIAMETER)` (the unit module is private).
    let diameter = (46.0 * CURRENT_DEVICE.dpi as f32 / 300.0).round() as i32;
    let progress = (value - min) / (max - min);
    let x = track.min.x + diameter / 2 + (progress * (track.width() as i32 - diameter) as f32).round() as i32;
    Point::new(x, track.center().y)
}

#[test]
fn dragging_the_size_slider_reflows_live_and_saves_on_release() {
    let mut e = env();
    let mut reader = open_panel(&mut e);
    let (min, max) = (e.context.settings.reader.min_font_size, e.context.settings.reader.max_font_size);
    let track = panel(&reader).slider_rect(SliderId::FontSize).unwrap();
    let pages = reader.page_position().unwrap().1;

    drive(&mut reader, &mut e, finger(FingerStatus::Down, slider_x(track, 14.0, (min, max))));
    // Moves before the reflow is due share one tick.
    drive(&mut reader, &mut e, finger(FingerStatus::Motion, slider_x(track, 15.0, (min, max))));
    drive(&mut reader, &mut e, finger(FingerStatus::Motion, slider_x(track, 16.0, (min, max))));
    assert_eq!(ticks(&e), 1, "one reflow for the burst");
    drive(&mut reader, &mut e, Event::LiveTextTick);
    assert_eq!(panel(&reader).state().text.font_size, DEFAULT_FONT_SIZE, "the panel shows the saved size until release");
    let live_pages = reader.page_position().unwrap().1;
    assert!(live_pages > pages, "bigger text, more pages: {live_pages} vs {pages}");
    assert_eq!(e.context.settings.reader.font_size, DEFAULT_FONT_SIZE, "nothing saved while dragging");

    // The next move waits for the gap after that reflow.
    drive(&mut reader, &mut e, finger(FingerStatus::Motion, slider_x(track, 13.0, (min, max))));
    assert_eq!(ticks(&e), 0, "too soon after the last reflow");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let tick = std::iter::from_fn(|| e.rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())).ok())
        .find(|evt| !matches!(evt, Event::LoadPixmap(..)));
    assert!(matches!(tick, Some(Event::LiveTextTick)), "the delayed reflow: {tick:?}");

    drive(&mut reader, &mut e, finger(FingerStatus::Up, slider_x(track, 13.0, (min, max))));
    assert_eq!(e.context.settings.reader.font_size, 13.0, "saved on release");
    assert_eq!(panel(&reader).state().text.font_size, 13.0);
    // A tick still on its way after the release does nothing.
    let at = reader.location();
    drive(&mut reader, &mut e, Event::LiveTextTick);
    assert_eq!(reader.location(), at);
}

#[test]
fn text_changes_keep_the_first_word_of_the_page() {
    let mut e = env();
    let mut reader = open_panel(&mut e);
    let start = reader.location();
    for size in [12.0, 14.5, 9.0, DEFAULT_FONT_SIZE] {
        drive(&mut reader, &mut e, Event::Slider(SliderId::FontSize, size, FingerStatus::Up));
        assert!(reader.location() <= start, "the word stays on the page at {size} pt");
    }
    assert_eq!(reader.location(), start, "back to the same text, back to the same page");
    drive(&mut reader, &mut e, Event::Slider(SliderId::LineHeight, 1.8, FingerStatus::Up));
    assert!(reader.location() <= start);
    drive(&mut reader, &mut e, Event::Slider(SliderId::LineHeight, 1.2, FingerStatus::Up));
    assert_eq!(reader.location(), start);
    // Margins too (compared with each other: the first layout's default margin assumes 300 dpi).
    drive(&mut reader, &mut e, Event::Slider(SliderId::MarginWidth, 3.0, FingerStatus::Up));
    let narrow = reader.location();
    assert!(narrow <= start);
    drive(&mut reader, &mut e, Event::Slider(SliderId::MarginWidth, 10.0, FingerStatus::Up));
    assert!(reader.location() <= start);
    drive(&mut reader, &mut e, Event::Slider(SliderId::MarginWidth, 3.0, FingerStatus::Up));
    assert_eq!(reader.location(), narrow);
    // A page turn starts from the new page.
    drive(&mut reader, &mut e, Event::Close(ViewId::TextPanel));
    drive(&mut reader, &mut e, Event::Page(CycleDir::Next));
    let next = reader.location();
    assert!(next > start);
    drive(&mut reader, &mut e, Event::Slider(SliderId::FontSize, 12.0, FingerStatus::Up));
    drive(&mut reader, &mut e, Event::Slider(SliderId::FontSize, DEFAULT_FONT_SIZE, FingerStatus::Up));
    assert_eq!(reader.location(), next);
}

#[test]
fn the_font_picker_and_cards_load_each_familys_face() {
    let mut e = env();
    let mut reader = open_panel(&mut e);
    assert!(e.context.fonts.has_reading_face("NV Literata"), "for the Bookish card");
    let family = panel(&reader).family_rect();
    drive(&mut reader, &mut e, tap_rect(family));
    assert!(locate_by_id(&reader, ViewId::FontFamilyMenu).is_some());
    assert!(e.context.fonts.has_reading_face("Libron"), "every listed family");
    assert!(!e.context.fonts.has_reading_face("No Such Family"));
}

#[test]
fn presets_apply_mark_changes_update_save_and_delete() {
    let mut e = env();
    let mut reader = open_panel(&mut e);
    // The default text is the Default preset.
    let state = panel(&reader).state().clone();
    assert_eq!(state.in_force(), Some((0, false)));
    assert!(!state.can_save() && !state.can_update());

    // Tap the Bookish card.
    let (_, card) = panel(&reader).cards()[1];
    drive(&mut reader, &mut e, tap_rect(card));
    let s = &e.context.settings.reader;
    assert_eq!(s.text_preset.as_deref(), Some("Bookish"));
    assert_eq!((s.font_family.as_str(), s.font_size, s.line_height, s.text_align), ("NV Literata", 10.5, 1.4, TextAlign::Justify));
    assert_eq!(panel(&reader).state().in_force(), Some((1, false)));

    // A change marks it.
    drive(&mut reader, &mut e, Event::Slider(SliderId::FontSize, 12.0, FingerStatus::Up));
    let state = panel(&reader).state().clone();
    assert_eq!(state.in_force(), Some((1, true)));
    assert_eq!(state.card_label(1), "Bookish*");
    assert!(state.can_save() && state.can_update());

    // Update saves it into the preset.
    let update = panel(&reader).update_rect();
    drive(&mut reader, &mut e, tap_rect(update));
    assert_eq!(e.context.settings.reader.text_presets[1].font_size, 12.0);
    assert_eq!(panel(&reader).state().card_label(1), "Bookish");

    // Save as new asks for a name, suggests one and puts the preset first.
    drive(&mut reader, &mut e, Event::Slider(SliderId::LineHeight, 1.7, FingerStatus::Up));
    let save = panel(&reader).save_rect();
    drive(&mut reader, &mut e, tap_rect(save));
    assert!(locate_by_id(&reader, ViewId::TextPresetName).is_some());
    drive(&mut reader, &mut e, Event::Submit(ViewId::TextPresetNameInput, " Airy ".into()));
    assert!(locate_by_id(&reader, ViewId::TextPresetName).is_none());
    let s = &e.context.settings.reader;
    assert_eq!(s.text_presets[0].name, "Airy");
    assert_eq!(s.text_presets[0].line_height, 1.7);
    assert_eq!(s.text_preset.as_deref(), Some("Airy"));
    assert_eq!(panel(&reader).state().in_force(), Some((0, false)));
    // Five presets: three cards and a pager.
    assert_eq!(panel(&reader).cards().len(), 3);
    assert!(panel(&reader).pager_rect().is_some());

    // Names are unique without regard to case.
    drive(&mut reader, &mut e, Event::Slider(SliderId::LineHeight, 1.5, FingerStatus::Up));
    drive(&mut reader, &mut e, Event::Submit(ViewId::TextPresetNameInput, "airy".into()));
    assert_eq!(e.context.settings.reader.text_presets.len(), 5);

    // Holding a card offers to delete it.
    let (index, card) = panel(&reader).cards()[0];
    drive(&mut reader, &mut e, Event::Gesture(GestureEvent::HoldFingerShort(card.center(), 0)));
    assert!(locate_by_id(&reader, ViewId::TextPresetMenu).is_some());
    drive(&mut reader, &mut e, Event::Select(EntryId::DeleteTextPreset(index)));
    let s = &e.context.settings.reader;
    assert_eq!(s.text_presets.len(), 4);
    assert_eq!(s.text_preset, None, "the deleted preset is no longer in force");
    assert!(panel(&reader).pager_rect().is_none());
}

#[test]
fn applying_the_preset_in_force_changes_nothing() {
    let mut e = env();
    let mut reader = open_panel(&mut e);
    let at = reader.location();
    let pages = reader.page_position().unwrap().1;
    drive(&mut reader, &mut e, Event::TextPreset(PresetAction::Apply(0)));
    assert_eq!((reader.location(), reader.page_position().unwrap().1), (at, pages));
    assert_eq!(e.context.settings.reader.text_preset.as_deref(), Some("Default"));
}

#[test]
fn changing_the_default_text_marks_the_default_preset() {
    let mut e = env();
    let mut reader = open_panel(&mut e);
    assert_eq!(e.context.settings.reader.text_preset, None, "never chosen");
    drive(&mut reader, &mut e, Event::Slider(SliderId::FontSize, 12.0, FingerStatus::Up));
    let state = panel(&reader).state().clone();
    assert_eq!(state.in_force(), Some((0, true)));
    assert_eq!(state.card_label(0), "Default*");
}
