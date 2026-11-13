//! Typographic niceties for article text: curly quotes, dashes and ellipses, and the language
//! the text is written in (so lines are hyphenated with the right patterns).

/// Curly quotes and apostrophes, `--` as an em dash and `...` as an ellipsis. `prev` is the
/// character before `text` (from an earlier text node), so quotes that start or end at an inline
/// tag still face the right way. Text without straight quotes, `--` or `...` is returned as is.
pub fn smarten(text: &str, prev: Option<char>) -> String {
    if !text.contains(['"', '\'']) && !text.contains("--") && !text.contains("...") {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 8);
    let mut before = prev;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        // An opening quote follows the start, a space, an opening bracket, a dash or another
        // opening quote; anything else closes.
        let opens = before.is_none_or(|b| b.is_whitespace() || "([{\u{2014}\u{2013}-\u{201c}\u{2018}".contains(b));
        let piece: String = match c {
            '-' if next == Some('-') => {
                let mut j = i;
                while chars.get(j) == Some(&'-') {
                    j += 1;
                }
                // A command-line option (" --verbose") keeps its hyphens.
                let option = before.is_none_or(|b| b.is_whitespace()) && chars.get(j).is_some_and(|n| n.is_alphanumeric());
                if option {
                    let run: String = chars[i..j].iter().collect();
                    i = j - 1;
                    run
                } else {
                    i = j - 1;
                    "\u{2014}".into()
                }
            }
            '.' if next == Some('.') && chars.get(i + 2) == Some(&'.') => {
                i += 2;
                "\u{2026}".into()
            }
            '"' => if opens { "\u{201c}" } else { "\u{201d}" }.into(),
            // An apostrophe unless it opens a quotation before a letter ('tis and '90s stay
            // apostrophes: a digit or nothing follows them).
            '\'' => if opens && next.is_some_and(|n| n.is_alphabetic()) { "\u{2018}" } else { "\u{2019}" }.into(),
            c => c.to_string(),
        };
        out.push_str(&piece);
        before = piece.chars().last();
        i += 1;
    }
    out
}

/// The language of `text` as the code the reader's hyphenation knows, when it can be told with
/// confidence (enough text, a reliable guess, a language with hyphenation patterns).
pub fn detect_language(text: &str) -> Option<&'static str> {
    if text.chars().count() < 200 {
        return None;
    }
    let info = whatlang::detect(text)?;
    if !info.is_reliable() {
        return None;
    }
    use whatlang::Lang::*;
    Some(match info.lang() {
        Eng => "en",
        Deu => "de",
        Fra => "fr",
        Spa => "es",
        Ita => "it",
        Por => "pt",
        Nld => "nl",
        Swe => "sv",
        Dan => "da",
        Nob => "nb",
        Fin => "fi",
        Pol => "pl",
        Rus => "ru",
        Ukr => "uk",
        Ces => "cs",
        Slk => "sk",
        Hun => "hu",
        Tur => "tr",
        Ell => "el",
        Ron => "ro",
        Hrv => "hr",
        Slv => "sl",
        Cat => "ca",
        Lit => "lt",
        Lav => "lv",
        Est => "et",
        Bul => "bg",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_dashes_and_ellipses() {
        assert_eq!(
            smarten("He said \"it's fine\" -- and left...", None),
            "He said \u{201c}it\u{2019}s fine\u{201d} \u{2014} and left\u{2026}"
        );
        assert_eq!(smarten("'quoted' and the '90s", None), "\u{2018}quoted\u{2019} and the \u{2019}90s");
        assert_eq!(smarten("\" after a tag", Some('a')), "\u{201d} after a tag");
        assert_eq!(smarten("\"Open", Some(' ')), "\u{201c}Open");
        assert_eq!(smarten("plain text", None), "plain text");
        assert_eq!(smarten("run it with --verbose", None), "run it with --verbose");
    }

    #[test]
    fn languages_with_enough_text() {
        assert_eq!(detect_language("Too short."), None);
        let en = "The quick brown fox jumps over the lazy dog while a long essay keeps unfolding across many pages, with enough words to tell which language this is.".repeat(2);
        assert_eq!(detect_language(&en), Some("en"));
        let de = "Der schnelle braune Fuchs springt \u{00fc}ber den faulen Hund, w\u{00e4}hrend ein langer Aufsatz sich \u{00fc}ber viele Seiten erstreckt und gen\u{00fc}gend W\u{00f6}rter enth\u{00e4}lt.".repeat(2);
        assert_eq!(detect_language(&de), Some("de"));
    }
}
