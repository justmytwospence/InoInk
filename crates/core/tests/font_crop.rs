//! Truncation with an ellipsis ends at a word boundary. Run through scripts/container-run.sh.

use plato_core::font::{font_from_style, Fonts, NORMAL_STYLE};

fn fonts() -> Fonts {
    std::env::set_current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).unwrap();
    Fonts::load().unwrap()
}

#[test]
fn crop_ends_at_a_word() {
    let mut fonts = fonts();
    let font = font_from_style(&mut fonts, &NORMAL_STYLE, 300);
    let text = "alpha beta gamma delta";
    let full = font.plan(text, None, None).width;
    let ellipsis = font.ellipsis.width;
    let target = font.plan("alpha beta gam", None, None).width + ellipsis;
    let cut = font.plan(text, Some(target), None).width;
    let expected = font.plan("alpha beta", None, None).width + ellipsis;
    assert!(cut < full);
    assert!((cut - expected).abs() <= 2, "cut at {cut}, expected about {expected}");
}

#[test]
fn a_single_long_word_is_still_cut() {
    let mut fonts = fonts();
    let font = font_from_style(&mut fonts, &NORMAL_STYLE, 300);
    let text = "supercalifragilisticexpialidocious";
    let w = font.plan(text, None, None).width;
    let ellipsis = font.ellipsis.width;
    let cut = font.plan(text, Some(w / 2), None).width;
    assert!(cut <= w / 2);
    assert!(cut > ellipsis);
}
