//! Whether a feed item is only a teaser of its article, and whether to fetch the full page.

use crate::store::FullText;

/// Feed text shorter than this (about 150 words) is treated as a teaser.
pub const STUB_CHARS: usize = 900;
/// Tries after which a feed's record decides on its own.
pub const LEARN_AFTER: i64 = 3;

/// The item's own signals: short text, or a trailing "read more"-style marker.
pub fn looks_like_stub(feed_text: &str) -> bool {
    let t = feed_text.trim();
    if t.chars().count() < STUB_CHARS {
        return true;
    }
    let l = t.to_lowercase();
    if ["\u{2026}", "...", "[\u{2026}]", "[...]", "(more\u{2026})"].iter().any(|m| l.ends_with(m)) {
        return true;
    }
    let core = l.trim_end_matches(|c: char| !c.is_alphanumeric());
    ["read more", "continue reading", "keep reading", "read the full article", "read the rest"].iter().any(|m| core.ends_with(m))
}

/// Whether to try the original page for an item of a feed with this record.
pub fn should_extract(mode: FullText, tries: i64, gains: i64, stub: bool) -> bool {
    match mode {
        FullText::Always => true,
        FullText::Never => false,
        FullText::Auto if tries >= LEARN_AFTER && gains == 0 => false,
        FullText::Auto if tries >= LEARN_AFTER && gains * 2 >= tries => true,
        FullText::Auto => stub,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_teasers() {
        assert!(looks_like_stub("A short summary."));
        let prose = "word ".repeat(250);
        assert!(!looks_like_stub(&prose));
        assert!(looks_like_stub(&format!("{prose}\u{2026} Continue reading \u{2192}")));
        assert!(looks_like_stub(&format!("{prose}[\u{2026}]")));
    }

    #[test]
    fn learning_decides_after_a_few_tries() {
        assert!(should_extract(FullText::Always, 9, 0, false));
        assert!(!should_extract(FullText::Never, 0, 0, true));
        assert!(should_extract(FullText::Auto, 0, 0, true));
        assert!(!should_extract(FullText::Auto, 3, 0, true));
        assert!(should_extract(FullText::Auto, 4, 2, false));
        assert!(!should_extract(FullText::Auto, 2, 0, false));
    }
}
