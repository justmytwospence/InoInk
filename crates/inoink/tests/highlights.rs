use inoink::highlight::*;

fn ann(start: i64, end: i64, text: &str) -> RemoteAnnotation {
    RemoteAnnotation { id: 1, start, end, text: text.into(), ..Default::default() }
}

fn words(s: &str) -> Vec<(&str, usize)> {
    s.split_whitespace().enumerate().map(|(i, w)| (w, i * 10)).collect()
}

#[test]
fn text_content_decodes_entities_and_skips_scripts() {
    let t = text_content("<p>Fish &amp; chips</p><script>var x = 1;</script><style>p{}</style><p>caf&eacute;<br>next</p>");
    assert_eq!(t, "Fish & chipscaf\u{e9}next");
}

#[test]
fn inoreader_offsets_are_utf16() {
    // The emoji is two UTF-16 units but one char: char offsets would land two places off.
    let html = "<p>\u{1F600} start. The chosen words here, then more text follows.</p>";
    let text = text_content(html);
    let byte = text.find("The chosen words").unwrap();
    let start = text[..byte].encode_utf16().count() as i64;
    let end = start + "The chosen words".encode_utf16().count() as i64;
    let q = quote_from_inoreader(html, &ann(start, end, "The chosen words"));
    assert_eq!(q.exact, "The chosen words");
    assert_eq!(q.prefix, "\u{1F600} start. ");
    assert_eq!(q.suffix, " here, then more text follows.");
    // Counting chars instead of UTF-16 units does not agree with the text, so there is no context.
    let chars = text[..byte].chars().count() as i64;
    let wrong = quote_from_inoreader(html, &ann(chars, chars + 16, "The chosen words"));
    assert!(wrong.prefix.is_empty() && wrong.suffix.is_empty());
}

#[test]
fn inoreader_bad_offsets_fall_back_to_text() {
    let html = "<p>Alpha beta gamma</p>";
    for (s, e) in [(0, 0), (-3, 4), (5, 400), (6, 10)] {
        let q = quote_from_inoreader(html, &ann(s, e, "Alpha beta"));
        assert_eq!(q, TextQuote { exact: "Alpha beta".into(), ..Default::default() }, "{s}..{e}");
    }
}

#[test]
fn locate_ignores_hyphenation_punctuation_and_case() {
    let w = [("Some", 0), ("Hyphen-", 5), ("ation", 12), ("matters.", 18), ("here", 30)];
    let q = TextQuote { exact: "hyphenation matters".into(), ..Default::default() };
    assert_eq!(locate(&w, &q, None), Some((5, 18)));
}

#[test]
fn locate_disambiguates_by_context() {
    let text = "the fox ran home . the fox ate lunch . the fox slept soundly";
    let w = words(text);
    let q = TextQuote { exact: "the fox".into(), prefix: "ate lunch.".into(), suffix: "slept soundly".into() };
    assert_eq!(locate(&w, &q, None), Some((w[10].1, w[11].1)));
    let q = TextQuote { exact: "the fox".into(), prefix: "".into(), suffix: "ate lunch".into() };
    assert_eq!(locate(&w, &q, None), Some((w[5].1, w[6].1)));
}

#[test]
fn locate_uses_hint_on_ties() {
    let w = words("one two one two one two");
    let q = TextQuote { exact: "one two".into(), ..Default::default() };
    assert_eq!(locate(&w, &q, None), Some((0, 10)), "earliest without a hint");
    assert_eq!(locate(&w, &q, Some(40)), Some((40, 50)));
    assert_eq!(locate(&w, &q, Some(18)), Some((20, 30)));
}

#[test]
fn locate_prefers_whole_words_but_accepts_partial_ones() {
    let w = words("concatenate the cat sat");
    let q = TextQuote { exact: "cat".into(), ..Default::default() };
    assert_eq!(locate(&w, &q, None), Some((20, 20)), "the word 'cat', not the one inside 'concatenate'");
    let q = TextQuote { exact: "catena".into(), ..Default::default() };
    assert_eq!(locate(&w, &q, None), Some((0, 0)), "a selection that starts inside a word still places");
}

#[test]
fn locate_missing_returns_none() {
    let w = words("nothing to see here");
    assert_eq!(locate(&w, &TextQuote { exact: "absent words".into(), ..Default::default() }, None), None);
    assert_eq!(locate(&w, &TextQuote { exact: "...".into(), ..Default::default() }, None), None);
    assert_eq!(locate(&[], &TextQuote { exact: "x".into(), ..Default::default() }, None), None);
}

#[test]
fn quote_from_words_clamps() {
    let w: Vec<&str> = "a b c d e f g h i j k l m n o p q r s t".split(' ').collect();
    let q = quote_from_words(&w, 10, 11);
    assert_eq!(q.exact, "k l");
    assert_eq!(q.prefix, "c d e f g h i j");
    assert_eq!(q.suffix, "m n o p q r s t");
    let q = quote_from_words(&w, 1, 99);
    assert_eq!((q.prefix.as_str(), q.suffix.as_str()), ("a", ""));
    assert_eq!(q.exact.split(' ').count(), 19);
    assert_eq!(quote_from_words(&w, 5, 2), TextQuote::default());
    assert_eq!(quote_from_words(&[], 0, 0), TextQuote::default());
}

#[test]
fn new_uid_is_32_hex_and_unique() {
    let ids: std::collections::BTreeSet<String> = (0..1000).map(|_| new_uid()).collect();
    assert_eq!(ids.len(), 1000);
    assert!(ids.iter().all(|u| u.len() == 32 && u.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())));
}

#[test]
fn rfc3339_known_values() {
    assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(rfc3339_utc(1_800_000_000), "2027-01-15T08:00:00Z");
}

fn export(uid: &str, article: &str, title: &str, author: &str, text: &str, note: &str) -> HighlightExport {
    HighlightExport {
        uid: uid.into(),
        article: article.into(),
        text: text.into(),
        note: note.into(),
        title: title.into(),
        author: author.into(),
        source_url: format!("https://blog.example/{article}"),
        feed_title: "Blog".into(),
        highlighted_at: 1_800_000_000,
        modified: 1_800_000_000,
        style: Default::default(),
        color: Default::default(),
    }
}

#[test]
fn markdown_groups_by_article() {
    let list = [
        export("1", "a", "First article", "Ann", "one", ""),
        export("2", "a", "First article", "Ann", "two", "a note"),
        export("3", "b", "Second article", "", "three", ""),
    ];
    let md = to_markdown(&list);
    assert_eq!(
        md,
        "# InoInk highlights\n\n## First article\n\nBlog \u{b7} Ann\nhttps://blog.example/a\n\n> one\n\n> two\n\na note\n\n\
         ## Second article\n\nBlog\nhttps://blog.example/b\n\n> three\n\n"
    );
}

#[test]
fn readwise_json_fields_and_truncation() {
    let long = "x".repeat(9000);
    let list = [export("1", "a", "T", "Ann", &long, "n"), export("2", "b", "U", "", "short", "")];
    let v = to_readwise_json(&list);
    let h = &v["highlights"];
    assert_eq!(h[0]["text"].as_str().unwrap().len(), 8191);
    assert_eq!((h[0]["title"].as_str(), h[0]["author"].as_str(), h[0]["note"].as_str()), (Some("T"), Some("Ann"), Some("n")));
    assert_eq!(h[0]["source_type"], "inoink");
    assert_eq!(h[0]["category"], "articles");
    assert_eq!(h[0]["highlighted_at"], "2027-01-15T08:00:00Z");
    assert_eq!(h[0]["source_url"], "https://blog.example/a");
    assert!(h[1].get("author").is_none() && h[1].get("note").is_none(), "empty author and note are omitted");
}

#[test]
fn style_and_colour_names_round_trip_and_default() {
    for st in HighlightStyle::ALL {
        assert_eq!(HighlightStyle::parse(st.as_str()), st);
    }
    for c in HighlightColor::ALL {
        assert_eq!(HighlightColor::parse(c.as_str()), c);
    }
    assert_eq!(HighlightStyle::parse("wavy"), HighlightStyle::Shaded);
    assert_eq!(HighlightColor::parse("teal"), HighlightColor::Gray);
    assert_eq!(HighlightColor::Gray.ink(), [0, 0, 0], "gray underlines are black");
    assert!(HighlightColor::Yellow.ink().iter().zip(HighlightColor::Yellow.tint()).all(|(i, t)| *i < t), "ink is darker than the tint");
}

#[test]
fn html_export_escapes_and_keeps_styles() {
    let mut a = export("1", "a", "Fish & <chips>", "Ann", "one\n\ntwo <b>", "a \"note\"");
    a.style = HighlightStyle::Dotted;
    a.color = HighlightColor::Blue;
    let mut b = export("2", "a", "Fish & <chips>", "Ann", "three", "");
    b.source_url = "javascript:alert(1)".into();
    let c = export("3", "b", "Second", "", "four", "");
    let html = to_html(&[a, b, c], 1_800_000_000);
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains("<h2>Fish &amp; &lt;chips&gt;</h2>"));
    assert_eq!(html.matches("<h2>").count(), 2, "grouped by article");
    assert!(html.contains("3 highlights \u{b7} 1 note \u{b7} exported 2027-01-15T08:00:00Z"));
    assert!(html.contains("class=\"dotted\" data-color=\"blue\""));
    assert!(html.contains("<p><span>one</span></p><p><span>two &lt;b&gt;</span></p>"));
    assert!(html.contains("<div class=\"note\"><p>a &quot;note&quot;</p></div>"));
    assert!(html.contains("<a href=\"https://blog.example/a\">"));
    assert!(!html.contains("javascript:"), "only web addresses become links");
    assert!(!html.contains("<b>"));
}

#[test]
fn export_file_names_sort_by_time() {
    assert_eq!(ExportFormat::Markdown.file_name(1_800_000_000), "inoink-highlights-2027-01-15-080000.md");
    assert_eq!(ExportFormat::Html.file_name(1_800_000_001), "inoink-highlights-2027-01-15-080001.html");
    let list = [export("1", "a", "T", "", "one", "")];
    assert_eq!(ExportFormat::Markdown.render(&list, 0), to_markdown(&list));
    assert!(ExportFormat::Html.render(&list, 0).contains("<blockquote"));
}
