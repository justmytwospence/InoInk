//! What the screen shows while the Kobo sleeps or is powered off (Settings > Sleep screen): a
//! status card, the current page, or the current page with a picture over it. The designs take
//! their cue from Kobalt's sleep screens (https://github.com/nicoverbruggen/kobalt, MIT).
//!
//! Everything is read when the screen is built, from what InoInk already has open: the database
//! for the counts and the last sync, the sync state in memory and the battery. Nothing is
//! downloaded or written.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use chrono::{Local, TimeZone};
use inoink::model::{ArticleQuery, Collection};
use inoink::runtime::SyncStatus;
use rand_core::RngCore;

use crate::battery::Status as BatteryStatus;
use crate::color::{Color, BLACK, GRAY06, GRAY10, WHITE};
use crate::context::Context;
use crate::device::CURRENT_DEVICE;
use crate::font::{font_from_style, Font, Fonts, MD_KIND, NORMAL_STYLE, DISPLAY_STYLE, TITLE_STYLE};
use crate::framebuffer::Framebuffer;
use crate::geom::{BorderSpec, CornerSpec, Rectangle};
use crate::unit::scale_by_dpi;
use crate::view::{BORDER_RADIUS_LARGE, THICKNESS_MEDIUM};

/// The folder next to InoInk's data folder (`.adds/inoink/screensaver` on the Kobo) that holds
/// the pictures for "Picture over the page".
pub const SCREENSAVER_DIRNAME: &str = "screensaver";

/// Pictures larger than this are skipped rather than decoded at suspend.
const MAX_PICTURE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_PICTURE_SIDE: u32 = 8192;

/// The numbers of the status card, gathered when the Kobo goes to sleep or powers off.
#[derive(Debug, Clone, PartialEq)]
pub struct SleepStatus {
    /// Unread articles in the Newsfeed, as its tab counts them.
    pub unread: usize,
    /// Articles in Read later.
    pub read_later: usize,
    /// "Synced today at 9:41 PM", or what stands in for it.
    pub sync: String,
    /// Local changes not sent to Inoreader yet.
    pub pending: usize,
    /// Battery level in percent and whether it is charging.
    pub battery: Option<(u8, BatteryStatus)>,
}

impl SleepStatus {
    pub fn gather(context: &mut Context) -> SleepStatus {
        let count = |c: Collection, unread: bool| {
            context.inoink.store()
                .and_then(|s| s.count_articles(&ArticleQuery { collection: Some(c), unread_only: unread, ..Default::default() }).ok())
                .unwrap_or(0)
        };
        let unread = count(Collection::All, true);
        let read_later = count(Collection::Starred, false);
        let ino = &context.inoink;
        // This session's last sync, else the last one recorded in the database (before a restart).
        let last = ino.last_sync.or_else(|| ino.store().and_then(|s| s.get_meta("last_cycle").ok().flatten()).and_then(|v| v.parse().ok()));
        let sync = sync_line(last, &ino.status, ino.handle.is_some(), ino.sandbox, Local::now().timestamp(), &context.settings.time_format);
        let pending = context.inoink.pending();
        let level = context.battery.capacity().ok().and_then(|v| v.first().copied());
        let state = context.battery.status().ok().and_then(|v| v.first().copied()).unwrap_or(BatteryStatus::Unknown);
        let battery = level.map(|l| (l.round().clamp(0.0, 100.0) as u8, state));
        SleepStatus { unread, read_later, sync, pending, battery }
    }

    /// The lines under the counts.
    pub fn details(&self) -> Vec<String> {
        let mut lines = vec![self.sync.clone()];
        if self.pending > 0 {
            lines.push(format!("{} to send", plural(self.pending, "change", "changes")));
        }
        if let Some((level, state)) = self.battery {
            lines.push(match state {
                BatteryStatus::Charging => format!("Battery {level}% \u{b7} charging"),
                BatteryStatus::Charged => "Battery charged".to_string(),
                _ => format!("Battery {level}%"),
            });
        }
        lines
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

/// When the last sync finished, in words that stay true however long the Kobo sleeps:
/// "Synced today at 9:41 PM", "Synced yesterday at ...", "Synced Nov 3 at ...".
pub fn sync_line(last_sync: Option<i64>, status: &SyncStatus, signed_in: bool, sandbox: bool, now: i64, time_format: &str) -> String {
    if sandbox {
        return "Snapshot sandbox".into();
    }
    let Some(t) = last_sync.or(match status {
        SyncStatus::Idle { last_sync } => *last_sync,
        _ => None,
    }) else {
        return match status {
            SyncStatus::NeedsSignIn => "Sign in required".into(),
            _ if !signed_in => "Offline library".into(),
            _ => "Not synced yet".into(),
        };
    };
    let (Some(then), Some(today)) = (Local.timestamp_opt(t, 0).single(), Local.timestamp_opt(now, 0).single()) else {
        return "Synced".into();
    };
    let clock = then.format(time_format).to_string();
    let days = (today.date_naive() - then.date_naive()).num_days();
    match days {
        0 => format!("Synced today at {clock}"),
        1 => format!("Synced yesterday at {clock}"),
        _ => format!("Synced {} at {clock}", then.format("%b %-d")),
    }
}

/// Where the screensaver pictures live: beside the data folder.
pub fn screensaver_dir(data_dir: &Path) -> PathBuf {
    match data_dir.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.join(SCREENSAVER_DIRNAME),
        _ => PathBuf::from(SCREENSAVER_DIRNAME),
    }
}

/// The PNG files in `dir`, sorted by name. Hidden files (such as macOS's `._` copies) are skipped.
pub fn pictures(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut list: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            !name.starts_with('.') && p.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case("png"))
        })
        .collect();
    list.sort();
    list
}

/// A picture decoded and scaled to fit the screen, as RGBA.
#[derive(Debug, Clone)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Picks one of the screensaver pictures at random and prepares it for `rect`. Unreadable files
/// are skipped; `None` when there is nothing usable.
pub fn random_picture(context: &mut Context, rect: &Rectangle) -> Option<Picture> {
    let mut list = pictures(&screensaver_dir(&context.inoink.data_dir));
    while !list.is_empty() {
        let i = (context.rng.next_u32() as usize) % list.len();
        let path = list.swap_remove(i);
        match load_picture(&path, rect.width(), rect.height()) {
            Ok(p) => return Some(p),
            Err(e) => eprintln!("sleep screen: skipping {}: {e}", path.display()),
        }
    }
    None
}

/// Decodes a PNG and scales it to fit within `max_width` x `max_height`, keeping its proportions.
pub fn load_picture(path: &Path, max_width: u32, max_height: u32) -> Result<Picture, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > MAX_PICTURE_BYTES {
        return Err("larger than 20 MB".into());
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let (w, h) = (reader.info().width, reader.info().height);
    if w == 0 || h == 0 || w > MAX_PICTURE_SIDE || h > MAX_PICTURE_SIDE {
        return Err(format!("unsupported size {w}x{h}"));
    }
    let (color, _) = reader.output_color_type();
    let mut buf = vec![0; reader.output_buffer_size().ok_or("too large")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let rgba = to_rgba(&buf[..info.buffer_size()], color, info.line_size, w, h)?;
    Ok(fit(&rgba, w, h, max_width, max_height))
}

fn to_rgba(buf: &[u8], color: png::ColorType, line: usize, w: u32, h: u32) -> Result<Vec<u8>, String> {
    let n = match color {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err("palette not expanded".into()),
    };
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        let row = &buf[y * line..y * line + w as usize * n];
        for px in row.chunks_exact(n) {
            out.extend_from_slice(&match n {
                1 => [px[0], px[0], px[0], 255],
                2 => [px[0], px[0], px[0], px[1]],
                3 => [px[0], px[1], px[2], 255],
                _ => [px[0], px[1], px[2], px[3]],
            });
        }
    }
    Ok(out)
}

/// Scales an RGBA image to fit `max_w` x `max_h`: averages (alpha-weighted) when shrinking,
/// repeats pixels when growing.
pub fn fit(rgba: &[u8], w: u32, h: u32, max_w: u32, max_h: u32) -> Picture {
    let scale = (max_w as f32 / w as f32).min(max_h as f32 / h as f32);
    let tw = ((w as f32 * scale).round() as u32).clamp(1, max_w.max(1));
    let th = ((h as f32 * scale).round() as u32).clamp(1, max_h.max(1));
    let mut out = Vec::with_capacity((tw * th * 4) as usize);
    for ty in 0..th {
        let y0 = (ty as u64 * h as u64 / th as u64) as u32;
        let y1 = (((ty + 1) as u64 * h as u64).div_ceil(th as u64) as u32).clamp(y0 + 1, h);
        for tx in 0..tw {
            let x0 = (tx as u64 * w as u64 / tw as u64) as u32;
            let x1 = (((tx + 1) as u64 * w as u64).div_ceil(tw as u64) as u32).clamp(x0 + 1, w);
            let (mut r, mut g, mut b, mut a, mut n) = (0u64, 0u64, 0u64, 0u64, 0u64);
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = ((y * w + x) * 4) as usize;
                    let pa = rgba[i + 3] as u64;
                    r += rgba[i] as u64 * pa;
                    g += rgba[i + 1] as u64 * pa;
                    b += rgba[i + 2] as u64 * pa;
                    a += pa;
                    n += 1;
                }
            }
            if a == 0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                out.extend_from_slice(&[(r / a) as u8, (g / a) as u8, (b / a) as u8, (a / n) as u8]);
            }
        }
    }
    Picture { width: tw, height: th, rgba: out }
}

/// Draws the picture centred in `rect`, blended over what is on screen, only within `clip`: when a
/// view underneath redraws part of the page, only that part gets the picture again, so
/// translucent pixels are never blended twice. With inverted colours the screen is inverted when
/// shown, so the picture is inverted here to keep its own colours.
pub fn draw_picture(fb: &mut dyn Framebuffer, picture: &Picture, rect: &Rectangle, clip: &Rectangle) {
    let dx = rect.min.x + (rect.width() as i32 - picture.width as i32) / 2;
    let dy = rect.min.y + (rect.height() as i32 - picture.height as i32) / 2;
    let area = rect![dx, dy, dx + picture.width as i32, dy + picture.height as i32];
    let Some(area) = area.intersection(clip).and_then(|a| a.intersection(&fb.rect())) else { return };
    let inverted = fb.inverted();
    for py in area.min.y..area.max.y {
        for px in area.min.x..area.max.x {
            let (x, y) = ((px - dx) as u32, (py - dy) as u32);
            let i = ((y * picture.width + x) * 4) as usize;
            let a = picture.rgba[i + 3];
            if a == 0 {
                continue;
            }
            let mut c = Color::Rgb(picture.rgba[i], picture.rgba[i + 1], picture.rgba[i + 2]);
            if inverted {
                c.invert();
            }
            fb.set_blended_pixel(px as u32, py as u32, c, a as f32 / 255.0);
        }
    }
}

/// The colours of a design: paper, ink, and the ink of labels.
fn scheme(dark: bool) -> (Color, Color, Color) {
    if dark { (BLACK, WHITE, GRAY10) } else { (WHITE, BLACK, GRAY06) }
}

/// The counts: bold sans, about 23 pt.
const COUNT_SIZE: u32 = 1500;

fn count_font<'a>(fonts: &'a mut Fonts, dpi: u16) -> &'a mut Font {
    let f = font_from_style(fonts, &TITLE_STYLE, dpi);
    f.set_size(COUNT_SIZE, dpi);
    f
}
const LABEL_TRACKING: f32 = 2.0;

/// The status card: the state ("Sleeping"), the two counts and the details, centred on a plain
/// page. Dark (white on black) when the Kobo is powered off.
pub fn render_card(fb: &mut dyn Framebuffer, fonts: &mut Fonts, rect: &Rectangle, title: &str, status: &SleepStatus, dark: bool) {
    let dpi = CURRENT_DEVICE.dpi;
    let (paper, ink, muted) = scheme(dark);
    fb.draw_rectangle(rect, paper);

    let (normal_lh, normal_asc, em) = {
        let f = font_from_style(fonts, &NORMAL_STYLE, dpi);
        (f.line_height(), f.ascender(), f.em() as i32)
    };
    let (count_lh, count_asc) = {
        let f = count_font(fonts, dpi);
        (f.line_height(), f.ascender())
    };
    let (label_lh, label_asc) = {
        let f = font_from_style(fonts, &MD_KIND, dpi);
        (f.line_height(), f.ascender())
    };

    let width = rect.width() as i32;
    let card_w = (width * 4 / 5).min(scale_by_dpi(520.0, dpi) as i32).max(width / 2);
    let pad = em;
    let details = status.details();

    // The title, in Plato's display face, scaled down to the card's width if needed.
    let mut title_size = DISPLAY_STYLE.size * 3 / 5;
    let (title_plan, title_h, title_asc) = {
        let f = font_from_style(fonts, &DISPLAY_STYLE, dpi);
        f.set_size(title_size, dpi);
        let mut plan = f.plan(title, None, None);
        if plan.width > card_w {
            title_size = (title_size as f32 * card_w as f32 / plan.width as f32) as u32;
            f.set_size(title_size, dpi);
            plan = f.plan(title, None, None);
        }
        (plan, f.line_height(), f.ascender())
    };

    let rows_h = count_lh + label_lh;
    let card_h = pad + rows_h + pad + details.len() as i32 * normal_lh + pad;
    let total = title_h + pad + card_h;
    let top = rect.min.y + (rect.height() as i32 - total) / 2;
    let left = rect.min.x + (width - card_w) / 2;

    {
        let f = font_from_style(fonts, &DISPLAY_STYLE, dpi);
        f.set_size(title_size, dpi);
        f.render(fb, ink, &title_plan, pt!(left + (card_w - title_plan.width) / 2, top + title_asc));
    }

    let card = rect![left, top + title_h + pad, left + card_w, top + title_h + pad + card_h];
    let radius = scale_by_dpi(BORDER_RADIUS_LARGE, dpi) as i32;
    let thick = scale_by_dpi(THICKNESS_MEDIUM, dpi) as u16;
    fb.draw_rounded_rectangle_with_border(&card, &CornerSpec::Uniform(radius), &BorderSpec { thickness: thick, color: ink }, &paper);

    // Two columns: unread and Read later.
    let col_w = card_w / 2;
    let counts = [(status.unread, "UNREAD"), (status.read_later, "READ LATER")];
    for (i, (n, label)) in counts.iter().enumerate() {
        let cx = left + col_w * i as i32;
        let y = card.min.y + pad;
        let f = count_font(fonts, dpi);
        let plan = f.plan(n.to_string(), Some(col_w - pad), None);
        f.render(fb, ink, &plan, pt!(cx + (col_w - plan.width) / 2, y + count_asc));
        let f = font_from_style(fonts, &MD_KIND, dpi);
        let mut plan = f.plan(label, Some(col_w - pad), None);
        plan.space_out(scale_by_dpi(LABEL_TRACKING, dpi) as i32);
        f.render(fb, muted, &plan, pt!(cx + (col_w - plan.width) / 2, y + count_lh + label_asc));
    }

    let sep_y = card.min.y + pad + rows_h + pad / 2;
    let sep = rect![left + pad, sep_y, left + card_w - pad, sep_y + thick.max(1) as i32];
    fb.draw_rectangle(&sep, muted);

    let f = font_from_style(fonts, &NORMAL_STYLE, dpi);
    for (i, line) in details.iter().enumerate() {
        let plan = f.plan(line, Some(card_w - 2 * pad), None);
        let y = sep_y + pad / 2 + i as i32 * normal_lh + normal_asc;
        f.render(fb, ink, &plan, pt!(left + (card_w - plan.width) / 2, y));
    }
}

/// The small "Sleeping" label drawn over the current page, centred near the bottom edge.
pub fn render_label(fb: &mut dyn Framebuffer, fonts: &mut Fonts, rect: &Rectangle, text: &str) {
    let dpi = CURRENT_DEVICE.dpi;
    let f = font_from_style(fonts, &NORMAL_STYLE, dpi);
    let plan = f.plan(text, Some(rect.width() as i32 / 2), None);
    let em = f.em() as i32;
    let (lh, asc) = (f.line_height(), f.ascender());
    let w = plan.width + 2 * em;
    let h = lh + em / 2;
    let left = rect.min.x + (rect.width() as i32 - w) / 2;
    let bottom = rect.max.y - 2 * em;
    let pill = rect![left, bottom - h, left + w, bottom];
    let thick = scale_by_dpi(THICKNESS_MEDIUM, dpi) as u16;
    fb.draw_rounded_rectangle_with_border(&pill, &CornerSpec::Uniform(h / 2), &BorderSpec { thickness: thick, color: BLACK }, &WHITE);
    f.render(fb, BLACK, &plan, pt!(left + em, pill.min.y + (h - lh) / 2 + asc));
}

#[cfg(test)]
mod tests {
    use super::*;

    const FMT: &str = "%-I:%M %p";

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        Local.with_ymd_and_hms(y, mo, d, h, mi, 0).single().unwrap().timestamp()
    }

    #[test]
    fn sync_line_names_the_day() {
        let now = at(2026, 11, 20, 22, 0);
        let idle = SyncStatus::Idle { last_sync: None };
        assert_eq!(sync_line(Some(at(2026, 11, 20, 21, 41)), &idle, true, false, now, FMT), "Synced today at 9:41 PM");
        assert_eq!(sync_line(Some(at(2026, 11, 19, 8, 5)), &idle, true, false, now, FMT), "Synced yesterday at 8:05 AM");
        assert_eq!(sync_line(Some(at(2026, 11, 3, 8, 5)), &idle, true, false, now, FMT), "Synced Nov 3 at 8:05 AM");
    }

    #[test]
    fn sync_line_keeps_the_last_sync_while_offline() {
        let now = at(2026, 11, 20, 22, 0);
        assert_eq!(sync_line(Some(at(2026, 11, 20, 21, 41)), &SyncStatus::Offline, true, false, now, FMT), "Synced today at 9:41 PM");
        let idle = SyncStatus::Idle { last_sync: Some(at(2026, 11, 20, 7, 0)) };
        assert_eq!(sync_line(None, &idle, true, false, now, FMT), "Synced today at 7:00 AM");
    }

    #[test]
    fn sync_line_without_a_sync() {
        let now = at(2026, 11, 20, 22, 0);
        assert_eq!(sync_line(None, &SyncStatus::NeedsSignIn, false, false, now, FMT), "Sign in required");
        assert_eq!(sync_line(None, &SyncStatus::Idle { last_sync: None }, false, false, now, FMT), "Offline library");
        assert_eq!(sync_line(None, &SyncStatus::Offline, true, false, now, FMT), "Not synced yet");
        assert_eq!(sync_line(Some(now), &SyncStatus::Offline, true, true, now, FMT), "Snapshot sandbox");
    }

    #[test]
    fn details_list_pending_changes_and_battery() {
        let mut s = SleepStatus { unread: 3, read_later: 1, sync: "Synced".into(), pending: 0, battery: Some((82, BatteryStatus::Discharging)) };
        assert_eq!(s.details(), vec!["Synced", "Battery 82%"]);
        s.pending = 1;
        s.battery = Some((40, BatteryStatus::Charging));
        assert_eq!(s.details(), vec!["Synced", "1 change to send", "Battery 40% \u{b7} charging"]);
        s.pending = 2;
        s.battery = None;
        assert_eq!(s.details(), vec!["Synced", "2 changes to send"]);
    }

    #[test]
    fn screensaver_dir_is_beside_the_data_dir() {
        assert_eq!(screensaver_dir(Path::new("/mnt/onboard/.adds/inoink/data")), PathBuf::from("/mnt/onboard/.adds/inoink/screensaver"));
        assert_eq!(screensaver_dir(Path::new("data")), PathBuf::from("screensaver"));
    }

    #[test]
    fn fit_keeps_proportions_and_averages_with_alpha() {
        // 4x2: left half opaque red, right half transparent.
        let mut rgba = Vec::new();
        for _ in 0..2 {
            for x in 0..4 {
                rgba.extend_from_slice(if x < 2 { &[255, 0, 0, 255] } else { &[0, 0, 255, 0] });
            }
        }
        let p = fit(&rgba, 4, 2, 2, 2);
        assert_eq!((p.width, p.height), (2, 1));
        assert_eq!(&p.rgba[..4], &[255, 0, 0, 255]);
        assert_eq!(p.rgba[7], 0);
        let big = fit(&rgba, 4, 2, 100, 100);
        assert_eq!((big.width, big.height), (100, 50));
        // Mixed opaque and transparent pixels keep the opaque colour, half covered.
        let half = fit(&rgba, 4, 2, 1, 1);
        assert_eq!(half.rgba, vec![255, 0, 0, 127]);
    }
}
