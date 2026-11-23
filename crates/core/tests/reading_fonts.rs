//! The bundled e-ink reading fonts: every family loads with its four styles, the list in the code
//! matches the files, and choosing one changes the article layout. Run through
//! scripts/container-run.sh (links MuPDF).

use plato_core::document::html::HtmlDocument;
use plato_core::document::{Document, Location};
use plato_core::font::{family_names, family_search_path, reading_family_names, FontFamily, BUNDLED_FONTS_PATH, BUNDLED_FONT_FAMILIES};
use plato_core::settings::{DEFAULT_FONT_FAMILY, DEFAULT_FONT_PATH};
use std::collections::BTreeSet;

fn repo_root() {
    // Fonts and CSS are loaded relative to the working directory.
    std::env::set_current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).unwrap();
}

#[test]
fn bundled_families_match_the_files() {
    repo_root();
    let found = family_names(BUNDLED_FONTS_PATH).unwrap();
    let listed: BTreeSet<String> = BUNDLED_FONT_FAMILIES.iter().map(|f| f.to_string()).collect();
    assert_eq!(found, listed, "BUNDLED_FONT_FAMILIES and fonts/reading disagree");
}

#[test]
fn every_bundled_family_has_four_styles() {
    repo_root();
    for family in BUNDLED_FONT_FAMILIES {
        let f = FontFamily::from_name(family, family_search_path(family, DEFAULT_FONT_PATH))
            .unwrap_or_else(|e| panic!("{family}: {e:#}"));
        let styles = [&f.regular, &f.italic, &f.bold, &f.bold_italic].map(|font| {
            assert_eq!(font.family_name(), Some(family));
            font.style_name().unwrap_or_default().to_string()
        });
        assert_eq!(styles, ["Regular", "Italic", "Bold", "Bold Italic"], "{family}");
    }
}

#[test]
fn families_resolve_to_their_directory() {
    assert_eq!(family_search_path(DEFAULT_FONT_FAMILY, "/mnt/onboard/fonts"), "fonts");
    assert_eq!(family_search_path("Libron", "/mnt/onboard/fonts"), BUNDLED_FONTS_PATH);
    assert_eq!(family_search_path("NV Literata", "/mnt/onboard/fonts"), BUNDLED_FONTS_PATH);
    assert_eq!(family_search_path("Bookerly", "/mnt/onboard/fonts"), "/mnt/onboard/fonts");
}

#[test]
fn the_menu_offers_default_bundled_and_user_families() {
    repo_root();
    let missing = reading_family_names("/nonexistent/fonts");
    let mut expected: BTreeSet<String> = BUNDLED_FONT_FAMILIES.iter().map(|f| f.to_string()).collect();
    expected.insert(DEFAULT_FONT_FAMILY.to_string());
    assert_eq!(missing, expected, "without a user font directory");

    let dir = tempfile::tempdir().unwrap();
    std::fs::copy("fonts/NotoSans-Regular.ttf", dir.path().join("NotoSans-Regular.ttf")).unwrap();
    // A user copy of a bundled family is listed once.
    std::fs::copy("fonts/reading/Libron-Regular.ttf", dir.path().join("Libron-Regular.ttf")).unwrap();
    let with_user = reading_family_names(dir.path());
    expected.insert("Noto Sans".to_string());
    assert_eq!(with_user, expected);
}

#[test]
fn choosing_a_bundled_family_changes_the_layout() {
    repo_root();
    let dir = tempfile::tempdir().unwrap();
    let words: String = (0..300).map(|i| format!("word{i} ")).collect();
    let html = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>T</title></head>\
         <body><p>{words}</p><p><em>{words}</em></p><p><strong>{words}</strong></p></body></html>"
    );
    let path = dir.path().join("v1.html");
    std::fs::write(&path, html).unwrap();
    let mut doc = HtmlDocument::new(&path).unwrap();
    doc.set_viewer_stylesheet("css/inoink.css");
    let first_page = |doc: &mut HtmlDocument| {
        doc.layout(1264, 1680, 11.0, 300);
        let (words, _) = doc.words(Location::Exact(0)).expect("words");
        assert!(!words.is_empty());
        words.iter().map(|w| (w.text.clone(), w.rect.min.x as i32, w.rect.max.x as i32)).collect::<Vec<_>>()
    };
    let default = first_page(&mut doc);
    for family in BUNDLED_FONT_FAMILIES {
        doc.set_font_family(family, family_search_path(family, DEFAULT_FONT_PATH));
        let page = first_page(&mut doc);
        assert_ne!(page, default, "{family} lays out like the default font");
    }
    doc.set_font_family(DEFAULT_FONT_FAMILY, family_search_path(DEFAULT_FONT_FAMILY, DEFAULT_FONT_PATH));
    assert_eq!(first_page(&mut doc), default, "back to the default font");
}
