//! HTML document behaviour InoInk relies on: rooted resources, image hit data, positions that
//! survive font and rotation changes. Run through scripts/container-run.sh (links MuPDF).

use plato_core::document::html::engine::ResourceFetcher;
use plato_core::document::html::HtmlDocument;
use plato_core::document::{Document, Location};
use std::path::PathBuf;

fn repo_root() {
    // Fonts, CSS and icons are loaded relative to the working directory.
    std::env::set_current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).unwrap();
}

#[test]
fn resources_are_confined_to_the_document_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("article");
    std::fs::create_dir_all(root.join("assets")).unwrap();
    std::fs::write(root.join("assets/a.png"), b"inside").unwrap();
    std::fs::write(dir.path().join("secret.txt"), b"outside").unwrap();
    std::os::unix::fs::symlink(dir.path().join("secret.txt"), root.join("assets/link.png")).unwrap();
    let mut r: PathBuf = root.clone();
    assert_eq!(r.fetch("assets/a.png").unwrap(), b"inside");
    assert!(r.fetch("../secret.txt").is_err(), "parent traversal");
    assert!(r.fetch("assets/../../secret.txt").is_err(), "nested traversal");
    assert!(r.fetch(dir.path().join("secret.txt").to_str().unwrap()).is_err(), "absolute path");
    assert!(r.fetch("/etc/passwd").is_err());
    assert!(r.fetch("assets/link.png").is_err(), "a symlink leaving the root");
    assert!(r.fetch("assets/missing.png").is_err());
}

fn article_with_images(dir: &std::path::Path) -> PathBuf {
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    let mut png = Vec::new();
    {
        use image::{ImageFormat, Rgb, RgbImage};
        RgbImage::from_pixel(400, 300, Rgb([200, 40, 40])).write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png).unwrap();
    }
    std::fs::write(dir.join("assets/a.png"), &png).unwrap();
    let words: String = (0..200).map(|i| format!("word{i} ")).collect();
    let html = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>T</title></head><body>\
         <h1>T</h1><p>{words}</p><img src=\"assets/a.png\" alt=\"a\"/><p>{words}</p><p>The end.</p></body></html>"
    );
    let path = dir.join("v1.html");
    std::fs::write(&path, html).unwrap();
    path
}

#[test]
fn images_are_reported_with_their_local_path_and_rectangle() {
    repo_root();
    let dir = tempfile::tempdir().unwrap();
    let path = article_with_images(dir.path());
    let mut doc = HtmlDocument::new(&path).unwrap();
    doc.set_viewer_stylesheet("css/inoink.css");
    doc.layout(1264, 1680, 11.0, 300);
    let mut found = None;
    let mut loc = Location::Exact(0);
    for _ in 0..20 {
        let Some((images, offset)) = doc.image_sources(loc) else { break };
        if let Some(i) = images.first() {
            found = Some(i.clone());
            break;
        }
        loc = Location::Next(offset);
    }
    let img = found.expect("an image on some page");
    assert_eq!(img.text, "assets/a.png");
    let r = img.rect;
    assert!(r.max.x > r.min.x && r.max.y > r.min.y, "non-empty rectangle {r:?}");
}

#[test]
fn text_offsets_resolve_after_font_size_and_rotation_changes() {
    repo_root();
    let dir = tempfile::tempdir().unwrap();
    let path = article_with_images(dir.path());
    let mut doc = HtmlDocument::new(&path).unwrap();
    doc.layout(1264, 1680, 11.0, 300);
    let size = doc.pages_count();
    assert!(size > 1000, "pages_count is the text size, not a page count");
    let offset = doc.resolve_location(Location::Exact(size / 2)).expect("middle of the document");
    for (w, h, fs) in [(1264, 1680, 14.0), (1680, 1264, 11.0), (1680, 1264, 16.0)] {
        doc.layout(w, h, fs, 300);
        let at = doc.resolve_location(Location::Exact(offset)).expect("offset still resolves");
        assert!(at <= offset, "the page containing the offset starts at or before it ({at} <= {offset})");
        let (words, _) = doc.words(Location::Exact(at)).unwrap();
        assert!(!words.is_empty(), "the page has text");
    }
}

#[test]
fn inline_formula_images_scale_with_the_font_and_stay_inline() {
    repo_root();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("assets")).unwrap();
    let mut png = Vec::new();
    {
        use image::{ImageFormat, Rgb, RgbImage};
        RgbImage::from_pixel(160, 40, Rgb([255, 255, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
    }
    std::fs::write(dir.path().join("assets/m.png"), &png).unwrap();
    let html = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>T</title></head><body>\
        <p>Before <img class=\"inoink-math\" src=\"assets/m.png\" alt=\"x\" style=\"display:inline;width:2em;vertical-align:-0.2em\"/> after.</p></body></html>";
    let path = dir.path().join("v1.html");
    std::fs::write(&path, html).unwrap();
    let mut doc = HtmlDocument::new(&path).unwrap();
    doc.set_viewer_stylesheet("css/inoink.css");

    let width_at = |doc: &mut HtmlDocument, pt: f32| {
        doc.layout(1264, 1680, pt, 300);
        let (images, _) = doc.image_sources(Location::Exact(0)).expect("images");
        let img = images.first().expect("the formula image").clone();
        let (words, _) = doc.words(Location::Exact(0)).expect("words");
        let before = words
            .iter()
            .find(|w| w.text.starts_with("Before"))
            .expect("word Before");
        assert!(
            img.rect.min.x >= before.rect.max.x,
            "inline: {:?} vs {:?}",
            img.rect,
            before.rect
        );
        (img.rect.max.x - img.rect.min.x) as f32
    };
    let w11 = width_at(&mut doc, 11.0);
    assert!((w11 - 2.0 * 11.0 * 300.0 / 72.0).abs() <= 3.0, "{w11}");
    let w22 = width_at(&mut doc, 22.0);
    assert!((w22 - 2.0 * w11).abs() <= 4.0, "{w22} vs {w11}");
}

/// The longest vertical run of dark pixels in any column.
fn longest_dark_column_run(p: &plato_core::framebuffer::Pixmap) -> u32 {
    let mut best = 0;
    for x in 0..p.width {
        let mut run = 0;
        for y in 0..p.height {
            if p.data[(y * p.width + x) as usize] < 100 {
                run += 1;
                best = best.max(run);
            } else {
                run = 0;
            }
        }
    }
    best
}

fn paragraphs_html(style: &str) -> String {
    let words: String = (0..60).map(|i| format!("word{i} ")).collect();
    let paras: String = (0..40).map(|_| format!("<p>{words}</p>")).collect();
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>T</title></head><body><div style=\"{style}\">{paras}</div></body></html>")
}

fn first_two_pages(html: &str) -> Vec<plato_core::framebuffer::Pixmap> {
    let mut doc = HtmlDocument::new_from_memory(html);
    doc.layout(1264, 1680, 11.0, 300);
    let (first, offset) = doc.pixmap(Location::Exact(0), 1.0, 1).expect("first page");
    let (second, _) = doc.pixmap(Location::Next(offset), 1.0, 1).expect("second page");
    vec![first, second]
}

#[test]
fn border_left_draws_a_continuous_line_on_every_page() {
    repo_root();
    let bordered = first_two_pages(&paragraphs_html("border-left-width: 2pt; border-left-color: #000; padding-left: 1em"));
    let plain = first_two_pages(&paragraphs_html("padding-left: 1em"));
    for (i, page) in bordered.iter().enumerate() {
        assert!(longest_dark_column_run(page) >= 400, "page {i} has a vertical line");
    }
    for (i, page) in plain.iter().enumerate() {
        assert!(longest_dark_column_run(page) < 100, "page {i} has no line without a border");
    }
}

#[derive(Debug, Clone)]
struct Word {
    text: String,
    rect: plato_core::geom::Rectangle,
}

fn xhtml(body: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>T</title></head><body>{body}</body></html>")
}

fn render_first(html: &str) -> (plato_core::framebuffer::Pixmap, Vec<Word>) {
    let mut doc = HtmlDocument::new_from_memory(html);
    doc.layout(1264, 1680, 11.0, 300);
    let (pixmap, _) = doc.pixmap(Location::Exact(0), 1.0, 1).expect("first page");
    let (words, _) = doc.words(Location::Exact(0)).expect("words");
    (pixmap, words.iter().map(|w| Word { text: w.text.clone(), rect: w.rect.to_rect() }).collect())
}

/// The contiguous column ranges where two renders differ, and the first and last differing rows.
fn diff_columns(a: &plato_core::framebuffer::Pixmap, b: &plato_core::framebuffer::Pixmap) -> (Vec<(i32, i32)>, (u32, u32)) {
    let mut columns = vec![false; a.width as usize];
    let (mut min_row, mut max_row) = (u32::MAX, 0);
    for y in 0..a.height {
        for x in 0..a.width {
            let i = (y * a.width + x) as usize;
            if a.data[i] != b.data[i] {
                columns[x as usize] = true;
                min_row = min_row.min(y);
                max_row = max_row.max(y);
            }
        }
    }
    let mut segments = Vec::new();
    let mut start = None;
    for (x, &on) in columns.iter().enumerate() {
        match (on, start) {
            (true, None) => start = Some(x as i32),
            (false, Some(s)) => {
                segments.push((s, x as i32));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        segments.push((s, columns.len() as i32));
    }
    (segments, (min_row, max_row))
}

fn decorated(tag: &str, decoration: &str, text: &str) -> String {
    xhtml(&format!("<p><{tag} href=\"x\" style=\"{decoration}\">{text}</{tag}></p>"))
}

#[test]
fn underline_spans_spaces_inside_a_link() {
    repo_root();
    let (on, words) = render_first(&decorated("a", "text-decoration: underline", "mama mama"));
    let (off, _) = render_first(&decorated("a", "text-decoration: none", "mama mama"));
    let (segments, (r0, r1)) = diff_columns(&on, &off);
    assert_eq!(segments.len(), 1, "{segments:?}");
    let (a, b) = segments[0];
    assert!((a - words[0].rect.min.x).abs() <= 2, "{a} vs {:?}", words[0].rect);
    assert!((b - words[1].rect.max.x).abs() <= 2, "{b} vs {:?}", words[1].rect);
    assert!(r1 - r0 < 6, "rows {r0}..={r1}");
}

#[test]
fn underline_skips_descenders() {
    repo_root();
    let (on, _) = render_first(&decorated("a", "text-decoration: underline", "gypsy gypsy"));
    let (off, _) = render_first(&decorated("a", "text-decoration: none", "gypsy gypsy"));
    let (segments, _) = diff_columns(&on, &off);
    assert!(segments.len() >= 4, "{segments:?}");
}

#[test]
fn line_through_is_solid() {
    repo_root();
    // Grey, so the line also shows where it crosses black stems.
    let (on, words) = render_first(&decorated("s", "text-decoration: line-through #888", "gypsy"));
    let (off, _) = render_first(&decorated("s", "text-decoration: none", "gypsy"));
    let (segments, (_, r1)) = diff_columns(&on, &off);
    assert_eq!(segments.len(), 1, "{segments:?}");
    let r = words[0].rect;
    assert!((r1 as i32) < r.max.y - r.height() as i32 / 4, "row {r1} vs {r:?}");
}

#[test]
fn decoration_color_is_used() {
    repo_root();
    let style = "text-decoration: underline; text-decoration-color: #888";
    let (on, _) = render_first(&decorated("a", style, "mama"));
    let (off, _) = render_first(&decorated("a", "text-decoration: none", "mama"));
    for (i, (p, q)) in on.data.iter().zip(off.data.iter()).enumerate() {
        if p != q {
            assert!((120..=150).contains(p), "pixel {i} is {p}");
        }
    }
}

/// Words grouped into lines by their top edge, top to bottom.
fn lines(words: &[Word]) -> Vec<Vec<Word>> {
    let mut lines: Vec<Vec<Word>> = Vec::new();
    for w in words {
        match lines.iter_mut().find(|l| l[0].rect.min.y == w.rect.min.y) {
            Some(l) => l.push(w.clone()),
            None => lines.push(vec![w.clone()]),
        }
    }
    lines.sort_by_key(|l| l[0].rect.min.y);
    lines
}

#[test]
fn opening_quotes_hang_into_the_margin() {
    repo_root();
    let words: String = (0..120).map(|i| format!(" word{i}")).collect();
    let para = |style: &str| xhtml(&format!("<p style=\"{style}text-align: justify\">\u{201c}Alpha{words}</p>"));
    let left = |l: &[Word]| l.iter().map(|w| w.rect.min.x).min().unwrap();
    let right = |l: &[Word]| l.iter().map(|w| w.rect.max.x).max().unwrap();

    let (_, hung) = render_first(&para("hanging-punctuation: first; "));
    let hung = lines(&hung);
    assert!(hung.len() > 3, "several lines");
    assert!(hung[0][0].text.starts_with('\u{201c}'));
    let others = hung[1..].iter().map(|l| left(l)).min().unwrap();
    assert!(left(&hung[0]) + 5 <= others, "{} vs {others}", left(&hung[0]));
    assert!((right(&hung[0]) - right(&hung[1])).abs() <= 3, "{} vs {}", right(&hung[0]), right(&hung[1]));

    let (_, plain) = render_first(&para(""));
    let plain = lines(&plain);
    let others = plain[1..].iter().map(|l| left(l)).min().unwrap();
    assert!((left(&plain[0]) - others).abs() <= 1, "{} vs {others}", left(&plain[0]));
}

#[test]
fn abbreviations_use_small_caps() {
    repo_root();
    let mut doc = HtmlDocument::new_from_memory(&xhtml("<p><abbr>NASA</abbr> <span>NASA</span></p>"));
    doc.set_viewer_stylesheet("css/inoink.css");
    doc.layout(1264, 1680, 11.0, 300);
    let (words, _) = doc.words(Location::Exact(0)).expect("words");
    let nasa: Vec<_> = words.iter().filter(|w| w.text == "NASA").map(|w| w.rect.to_rect()).collect();
    assert_eq!(nasa.len(), 2, "{:?}", words.iter().map(|w| &w.text).collect::<Vec<_>>());
    let (a, b) = (nasa[0].width() as f32, nasa[1].width() as f32);
    assert!(a < 0.9 * b, "{a} vs {b}");
}
