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
    assert_eq!(t.comments[0].date, "2026-11-11");
    assert_eq!(t.article_url, None);
    assert_eq!(t.post_html.as_deref(), Some("<p>Self post</p>"));
}

#[test]
fn renders_comments_well_formed() {
    let t = Thread {
        title: "T".into(),
        article_url: None,
        post_html: None,
        count: 2,
        comments: vec![
            Comment {
                author: "<b>x</b>".into(),
                html: "<p>hi <img src=\"https://i.example/a.png\" alt=\"pic\"></p>".into(),
                date: "2026-11-11".into(),
                depth: 0,
            },
            Comment { author: "deep".into(), html: "<p>reply & more</p>".into(), date: String::new(), depth: 7 },
        ],
    };
    let d = Discussion { site: Site::Lobsters, id: "abc".into() };
    let p = render_comments(&t, &d, "A <title>");
    assert_well_formed(&p.html);
    assert!(p.html.contains("inoink-article:") && p.html.contains("inoink-d5"));
    assert!(p.html.contains("&lt;b&gt;x&lt;/b&gt;"));
    assert!(!p.html.contains("<img") && p.html.contains("pic"));
    assert!(p.assets.is_empty());
}
