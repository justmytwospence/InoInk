use inoink::content::discussion::*;

fn assert_well_formed(html: &str) {
    let mut stack: Vec<String> = Vec::new();
    let mut i = 0;
    let b = html.as_bytes();
    while i < b.len() {
        if b[i] == b'<' {
            let end = html[i..].find('>').expect("unterminated tag") + i;
            let inner = &html[i + 1..end];
            if inner.starts_with('?') || inner.starts_with('!') {
            } else if let Some(name) = inner.strip_prefix('/') {
                assert_eq!(stack.pop().as_deref(), Some(name.trim()), "mismatched close in {html}");
            } else if !inner.ends_with('/') {
                stack.push(inner.split_whitespace().next().unwrap().to_string());
            }
            i = end + 1;
        } else {
            assert_ne!(b[i], b'>', "stray > in text");
            i += 1;
        }
    }
    assert!(stack.is_empty(), "unclosed: {stack:?}");
}

#[test]
fn detects_hn_and_lobsters_links() {
    let d = detect("https://example.com/a", "<a href=\"https://news.ycombinator.com/item?id=50458900\">Comments</a>").unwrap();
    assert_eq!((d.site, d.id.as_str()), (Site::HackerNews, "50458900"));
    let d = detect(
        "https://x.dev/p",
        "<p>Comments URL: <a href=\"https://news.ycombinator.com/item?id=50459485\">https://news.ycombinator.com/item?id=50459485</a></p>",
    )
    .unwrap();
    assert_eq!(d.id, "50459485");
    let d = detect("https://michael.bacarella.com/2026/11/10/x/", "<p><a href=\"https://lobste.rs/s/ii1gzp/how_false\">Comments</a></p>")
        .unwrap();
    assert_eq!((d.site, d.id.as_str()), (Site::Lobsters, "ii1gzp"));
    let d = detect("https://lobste.rs/s/m43bas/how_do_you", "<p>text</p>").unwrap();
    assert_eq!((d.site, d.id.as_str()), (Site::Lobsters, "m43bas"));
    assert_eq!(d.api_url(), "https://lobste.rs/s/m43bas.json");
    assert!(detect("https://blog.example/post", "<p>plain</p>").is_none());
    assert!(detect("https://news.ycombinator.com/item?id=", "").is_none());
    assert!(is_discussion_url("https://news.ycombinator.com/item?id=1"));
    assert!(!is_discussion_url("https://blog.example/"));
}

#[test]
fn parses_hn_tree_in_order_with_depths() {
    let json = r#"{"type":"story","title":"T","url":"https://a.example/x","text":null,"children":[
      {"type":"comment","author":"a","text":"<p>1</p>","created_at_i":1700000000,"children":[
         {"type":"comment","author":"b","text":"<p>2</p>","created_at_i":1700000001,"children":[
            {"type":"comment","author":null,"text":null,"children":[
               {"type":"comment","author":"c","text":"<p>3</p>","created_at_i":1700000002,"children":[]}]}]}]},
      {"type":"comment","author":null,"text":null,"children":[]},
      {"type":"comment","author":"d","text":"<p>4</p>","created_at_i":1700000003,"children":[]}]}"#;
    let t = parse_thread(Site::HackerNews, json.as_bytes()).unwrap();
    let seq: Vec<(&str, u32)> = t.comments.iter().map(|c| (c.author.as_str(), c.depth)).collect();
    assert_eq!(seq, vec![("a", 0), ("b", 1), ("[deleted]", 2), ("c", 3), ("d", 0)]);
    assert_eq!(t.count, 4);
    assert_eq!(t.article_url.as_deref(), Some("https://a.example/x"));
    let ask =
        parse_thread(Site::HackerNews, br#"{"type":"story","title":"Ask HN","url":null,"text":"<p>Question</p>","children":[]}"#).unwrap();
    assert_eq!(ask.article_url, None);
    assert_eq!(ask.post_html.as_deref(), Some("<p>Question</p>"));
    assert!(parse_thread(Site::HackerNews, b"nope").is_err());
}

#[test]
fn parses_lobsters_flat_list() {
    let json = r#"{"title":"T","url":"","description":"<p>Self post</p>","comments":[
      {"depth":0,"commenting_user":"a","comment":"<p>x</p>","created_at":"2026-11-11T12:16:50.200-06:00","is_deleted":false},
      {"depth":1,"commenting_user":"b","comment":"<p>y</p>","created_at":"2026-11-11T13:00:00.000-06:00","is_deleted":true},
      {"depth":1,"commenting_user":"c","comment":"<p>z</p>","created_at":"2026-11-12T13:00:00.000-06:00","is_deleted":false}]}"#;
    let t = parse_thread(Site::Lobsters, json.as_bytes()).unwrap();
    assert_eq!(t.count, 2);
    assert_eq!(t.comments.iter().map(|c| c.depth).collect::<Vec<_>>(), vec![0, 1, 1]);
    assert_eq!(t.comments[1].author, "[deleted]");
    assert_eq!(t.comments[0].time, 1794421010);
    assert_eq!(t.article_url, None);
    assert_eq!(t.post_html.as_deref(), Some("<p>Self post</p>"));
}

#[test]
fn renders_comments_well_formed() {
    let t = Thread {
        title: "T".into(),
        article_url: None,
        post_html: None,
        submitter: None,
        count: 2,
        comments: vec![
            Comment {
                author: "<b>x</b>".into(),
                html: "<p>hi <img src=\"https://i.example/a.png\" alt=\"pic\"></p>".into(),
                time: 1794000000,
                depth: 0,
            },
            Comment { author: "deep".into(), html: "<p>reply & more</p>".into(), time: 0, depth: 7 },
        ],
    };
    let d = Discussion { site: Site::Lobsters, id: "abc".into() };
    let p = render_comments(&t, &d, "A <title>", 1794003600);
    assert_well_formed(&p.html);
    assert!(p.html.contains("inoink-article:") && p.html.contains("inoink-replies"));
    assert!(p.html.contains("&lt;b&gt;x&lt;/b&gt;"));
    assert!(!p.html.contains("<img") && p.html.contains("pic"));
    assert!(p.assets.is_empty());
}

#[test]
fn comments_cap_nesting_mark_the_poster_and_link_threads() {
    let json = r#"{"type":"story","author":"op","title":"T","url":null,"text":null,"children":[
      {"type":"comment","author":"alice","text":"<p>a</p>","created_at_i":1794000000,"children":[
        {"type":"comment","author":"bob","text":"<p>b</p>","created_at_i":1794000060,"children":[
          {"type":"comment","author":"op","text":"<p>c</p>","created_at_i":1794000120,"children":[
            {"type":"comment","author":"erin2","text":"<p>f</p>","created_at_i":1794000180,"children":[]}]}]},
        {"type":"comment","author":"dan","text":"<p>d</p>","created_at_i":1794000240,"children":[]}]},
      {"type":"comment","author":"erin","text":"<p>e</p>","created_at_i":1794000300,"children":[]}]}"#;
    let t = parse_thread(Site::HackerNews, json.as_bytes()).unwrap();
    assert_eq!(t.submitter.as_deref(), Some("op"));
    let p = render_comments(&t, &Discussion { site: Site::HackerNews, id: "1".into() }, "T", 1794003600);
    assert_well_formed(&p.html);
    let h = &p.html;
    assert_eq!(h.matches("id=\"inoink-t").count(), 2);
    assert!(h.contains("<b>alice</b> \u{b7} 1 h \u{b7} 4 replies"));
    assert!(h.contains("4 replies \u{b7} <a href=\"#inoink-t2\">Skip \u{203a}</a>"));
    assert!(!h.contains("#inoink-t3"), "no skip after the last thread");
    assert!(h.contains("<b>bob</b> \u{b7}"), "no parent name at depth 1");
    assert!(!h.contains("<b>bob</b> \u{21b3}"));
    assert!(h.contains("<b>op</b> OP \u{b7}"), "the poster is marked, no parent at depth 2");
    assert!(h.contains("<b>erin2</b> \u{21b3} op"), "depth 3 names its parent");
    assert_eq!(h.matches("<div class=\"inoink-replies\">").count(), 2, "two indent levels, one container each");
    assert_eq!(h.matches("<div class=\"inoink-thread\">").count(), 2);
}

#[test]
fn age_text_buckets() {
    let now = 1_800_000_000;
    assert_eq!(age_text(0, now), "");
    assert_eq!(age_text(now + 5, now), "now");
    assert_eq!(age_text(now - 59, now), "now");
    assert_eq!(age_text(now - 60, now), "1 min");
    assert_eq!(age_text(now - 7200, now), "2 h");
    assert_eq!(age_text(now - 172_800, now), "2 d");
    assert_eq!(age_text(now - 40 * 86_400, now), "1 mo");
    assert_eq!(age_text(now - 400 * 86_400, now), "1 y");
}
