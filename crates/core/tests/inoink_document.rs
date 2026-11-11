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
