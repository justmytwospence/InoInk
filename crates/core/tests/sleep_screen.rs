//! Sleep and power-off screens (Settings > Sleep screen): the setting, the menu, what each
//! choice draws and the numbers on the status card. Run through scripts/container-run.sh
//! (links MuPDF).

use inoink::runtime::{SyncStatus, Update};
use inoink::store::{ArticleRecord, Store};
use plato_core::battery::FakeBattery;
use plato_core::color::{BLACK, WHITE};
use plato_core::context::Context;
use plato_core::font::Fonts;
use plato_core::framebuffer::{Framebuffer, Pixmap};
use plato_core::frontlight::LightLevels;
use plato_core::settings::{IntermKind, Settings, SleepScreen};
use plato_core::view::inoink::actions::InoInkEntry;
use plato_core::view::inoink::manage;
use plato_core::view::inoink::sleep::{self, SleepStatus};
use plato_core::view::inoink::state::InoInk;
use plato_core::view::intermission::Intermission;
use plato_core::view::{EntryId, EntryKind, View};
use plato_core::rect;
use std::path::Path;
use std::sync::mpsc;

const W: u32 = 632;
const H: u32 = 840;

fn rec(id: u64, starred: bool) -> ArticleRecord {
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
        timestamp_usec: id as i64 * 1_000_000_000,
        published: id as i64 * 1000,
        summary_html: format!("<p>Body {id}</p>"),
        raw_json: "{}".into(),
        categories: cats,
        ..Default::default()
    }
}

struct Env {
    dir: tempfile::TempDir,
    context: Context,
}

/// Three articles, one of them in Read later. The data folder is `<tmp>/data`, so the
/// screensaver folder is `<tmp>/screensaver`.
fn env() -> Env {
    std::env::set_current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    {
        let mut store = Store::open(&data.join("inoink.db"), "local").unwrap();
        for (id, starred) in [(1u64, true), (2, false), (3, false)] {
            store.upsert_article(&rec(id, starred), 0).unwrap();
        }
        store.checkpoint().unwrap();
    }
    let mut settings = Settings::default();
    settings.inoink.data_dir = data;
    let (tx, _rx) = mpsc::channel();
    let inoink = InoInk::open(&settings.inoink, tx, false).unwrap();
    let fb = Box::new(Pixmap::new(W, H, 3));
    let context = Context::new(fb, None, inoink, settings, Fonts::load().unwrap(), Box::new(FakeBattery::new()), Box::new(LightLevels::default()), Box::new(0u16));
    Env { dir, context }
}

/// Fills the screen with a mid grey "page", then draws the screen for `kind` over it.
fn draw(e: &mut Env, kind: IntermKind) -> Pixmap {
    let mut fb = Pixmap::new(W, H, 3);
    let rect = fb.rect();
    fb.draw_rectangle(&rect, plato_core::color::GRAY08);
    let interm = Intermission::new(rect, kind, &mut e.context);
    interm.render(&mut fb, rect, &mut e.context.fonts);
    fb
}

fn px(fb: &Pixmap, x: u32, y: u32) -> [u8; 3] {
    fb.get_pixel(x, y).rgb()
}

fn write_png(path: &Path, w: u32, h: u32, rgba: [u8; 4]) {
    let file = std::fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().unwrap();
    let data: Vec<u8> = (0..w * h).flat_map(|_| rgba).collect();
    writer.write_image_data(&data).unwrap();
}

#[test]
fn the_status_card_is_the_default_and_the_setting_reads_back() {
    assert_eq!(Settings::default().inoink.sleep_screen, SleepScreen::StatusCard);
    let s: Settings = toml::from_str("[inoink]\nsleep-screen = \"current-page\"\n").unwrap();
    assert_eq!(s.inoink.sleep_screen, SleepScreen::CurrentPage);
    let s: Settings = toml::from_str("[inoink]\ncache-budget-mb = 256\n").unwrap();
    assert_eq!(s.inoink.sleep_screen, SleepScreen::StatusCard, "older settings get the default");
    let out = toml::to_string(&Settings::default()).unwrap();
    assert!(out.contains("sleep-screen = \"status-card\""), "{out}");
}

#[test]
fn settings_offer_every_sleep_screen_and_apply_the_choice() {
    let mut e = env();
    let entries = manage::sleep_screen_entries(&e.context);
    let radios: Vec<(String, bool)> = entries
        .iter()
        .filter_map(|k| match k {
            EntryKind::RadioButton(l, EntryId::InoInk(InoInkEntry::SetSleepScreen(_)), on) => Some((l.clone(), *on)),
            _ => None,
        })
        .collect();
    assert_eq!(
        radios,
        vec![("Status card".into(), true), ("Current page".into(), false), ("Picture over the page".into(), false), ("Logo".into(), false)]
    );
    // No pictures yet: the menu says where they go.
    assert!(entries.iter().any(|k| matches!(k, EntryKind::Message(m, _) if m.contains(".adds/inoink/screensaver"))));

    manage::apply(&mut e.context, &InoInkEntry::SetSleepScreen(SleepScreen::Picture), None).unwrap();
    assert_eq!(e.context.settings.inoink.sleep_screen, SleepScreen::Picture);
    assert_eq!(e.context.inoink.settings.sleep_screen, SleepScreen::Picture);

    std::fs::create_dir_all(e.dir.path().join("screensaver")).unwrap();
    write_png(&e.dir.path().join("screensaver/a.png"), 4, 4, [0, 0, 0, 255]);
    let entries = manage::sleep_screen_entries(&e.context);
    assert!(!entries.iter().any(|k| matches!(k, EntryKind::Message(..))));

    let main = manage::main_menu_entries(&e.context);
    let settings = main.iter().find_map(|k| match k {
        EntryKind::SubMenu(l, sub) if l == "Settings" => Some(sub),
        _ => None,
    }).unwrap();
    assert!(settings.iter().any(|k| matches!(k, EntryKind::SubMenu(l, _) if l == "Sleep screen")));
}

#[test]
fn the_card_counts_unread_and_read_later_and_keeps_the_last_sync() {
    let mut e = env();
    assert_eq!(SleepStatus::gather(&mut e.context).sync, "Offline library");
    // After a restart: the last sync recorded in the database.
    e.context.inoink.store_mut().unwrap().set_meta("last_cycle", "1700000000").unwrap();
    let s = SleepStatus::gather(&mut e.context).sync;
    assert!(s.starts_with("Synced Nov 1"), "{s}");
    e.context.inoink.apply(&Update::Status(SyncStatus::Idle { last_sync: Some(1_800_000_000) }));
    e.context.inoink.apply(&Update::Status(SyncStatus::Offline));
    let s = SleepStatus::gather(&mut e.context);
    assert_eq!((s.unread, s.read_later, s.pending), (3, 1, 0));
    assert!(s.sync.starts_with("Synced "), "{}", s.sync);
    assert_eq!(s.battery, Some((50, plato_core::battery::Status::Discharging)));
    assert_eq!(s.details().last().unwrap(), "Battery 50%");
}

#[test]
fn the_status_card_covers_the_page_and_power_off_is_dark() {
    let mut e = env();
    let fb = draw(&mut e, IntermKind::Suspend);
    assert_eq!(px(&fb, 1, 1), WHITE.rgb(), "plain paper");
    assert!((0..H).any(|y| px(&fb, W / 2, y) == BLACK.rgb()), "the card is drawn");

    // Powering off shows the dark card whatever is chosen for sleep, except Plato's logo.
    for choice in [SleepScreen::StatusCard, SleepScreen::CurrentPage, SleepScreen::Picture] {
        e.context.settings.inoink.sleep_screen = choice;
        let fb = draw(&mut e, IntermKind::PowerOff);
        assert_eq!(px(&fb, 1, 1), BLACK.rgb(), "{choice:?}");
        assert!((0..H).any(|y| px(&fb, W / 2, y) == WHITE.rgb()), "{choice:?}");
    }
}

#[test]
fn the_current_page_stays_with_a_label() {
    let mut e = env();
    e.context.settings.inoink.sleep_screen = SleepScreen::CurrentPage;
    let fb = draw(&mut e, IntermKind::Suspend);
    let grey = plato_core::color::GRAY08.rgb();
    assert_eq!(px(&fb, 1, 1), grey, "the page is left alone");
    assert_eq!(px(&fb, W / 2, H / 2), grey);
    let bottom: Vec<_> = (H * 3 / 4..H).map(|y| px(&fb, W / 2, y)).collect();
    assert!(bottom.contains(&WHITE.rgb()) && bottom.contains(&BLACK.rgb()), "the label near the bottom");
}

#[test]
fn a_picture_is_drawn_over_the_page() {
    let mut e = env();
    e.context.settings.inoink.sleep_screen = SleepScreen::Picture;
    let grey = plato_core::color::GRAY08.rgb();
    // Without pictures it is the current page.
    let fb = draw(&mut e, IntermKind::Suspend);
    assert_eq!(px(&fb, W / 2, H / 2), grey);

    let dir = sleep::screensaver_dir(&e.context.inoink.data_dir);
    assert_eq!(dir, e.dir.path().join("screensaver"));
    std::fs::create_dir_all(&dir).unwrap();
    // A square, opaque red picture: it fits the width and is centred.
    write_png(&dir.join("red.png"), 10, 10, [255, 0, 0, 255]);
    std::fs::write(dir.join("._red.png"), b"not a png").unwrap();
    std::fs::write(dir.join("notes.txt"), b"ignored").unwrap();
    assert_eq!(sleep::pictures(&dir), vec![dir.join("red.png")]);
    let fb = draw(&mut e, IntermKind::Suspend);
    assert_eq!(px(&fb, W / 2, H / 2), [255, 0, 0]);
    assert_eq!(px(&fb, 1, 1), grey, "outside the picture the page shows");

    // A transparent picture leaves the page visible through it.
    std::fs::remove_file(dir.join("red.png")).unwrap();
    write_png(&dir.join("clear.png"), 10, 10, [255, 0, 0, 0]);
    let fb = draw(&mut e, IntermKind::Suspend);
    assert_eq!(px(&fb, W / 2, H / 2), grey);

    // A translucent one is blended once, also when part of the screen is drawn again.
    std::fs::remove_file(dir.join("clear.png")).unwrap();
    write_png(&dir.join("shade.png"), 10, 10, [0, 0, 0, 128]);
    let mut fb = Pixmap::new(W, H, 3);
    let rect = fb.rect();
    fb.draw_rectangle(&rect, plato_core::color::GRAY08);
    let interm = Intermission::new(rect, IntermKind::Suspend, &mut e.context);
    interm.render(&mut fb, rect, &mut e.context.fonts);
    let shaded = px(&fb, W / 2, H / 2);
    assert!(shaded[0] < grey[0] && shaded[0] > 0, "{shaded:?}");
    let corner = rect![0, H as i32 / 2 - 20, 20, H as i32 / 2 + 20];
    interm.render(&mut fb, corner, &mut e.context.fonts);
    assert_eq!(px(&fb, W / 2, H / 2), shaded);
    std::fs::remove_file(dir.join("shade.png")).unwrap();

    // A broken file is skipped.
    std::fs::write(dir.join("broken.png"), b"not a png").unwrap();
    let rect = e.context.fb.rect();
    assert!(sleep::random_picture(&mut e.context, &rect).is_none());
}

#[test]
fn the_logo_choice_keeps_platos_screen() {
    let mut e = env();
    e.context.settings.inoink.sleep_screen = SleepScreen::Plato;
    let fb = draw(&mut e, IntermKind::Suspend);
    assert_eq!(px(&fb, 1, 1), WHITE.rgb());
    let fb = draw(&mut e, IntermKind::PowerOff);
    assert_eq!(px(&fb, 1, 1), BLACK.rgb());
}
