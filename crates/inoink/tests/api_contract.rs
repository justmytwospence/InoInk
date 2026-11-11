mod common;
use common::*;
use inoink::api::auth;
use inoink::api::*;
use inoink::error::Error;
use inoink::model::*;
use serde_json::json;

fn item_json(id: &str, extra: serde_json::Value) -> serde_json::Value {
    let mut v = json!({
        "crawlTimeMsec": "1618211779000",
        "timestampUsec": "1618211779000000",
        "id": id,
        "categories": ["user/1005921515/state/com.google/reading-list", "user/1005921515/state/com.google/read", "user/1005921515/label/Tech"],
        "title": "Windows and Linux devices",
        "published": 1617969599,
        "canonical": [{"href": "https://arstechnica.com/?p=1755573"}],
        "summary": {"direction": "ltr", "content": "<p>hello</p>"},
        "author": "Dan",
        "origin": {"streamId": "feed/http://feeds.arstechnica.com/arstechnica/gadgets", "title": "Ars", "htmlUrl": "http://arstechnica.com/"},
        "annotations": [{"id": 1, "start": 402, "end": 548, "text": "x", "note": "Check your firewall!"}],
        "summaries": [{"id": 9, "prompt_name": "One sentence summary", "summary": "<p>S</p>"}],
        "someFutureField": {"keep": true}
    });
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        for (k, val) in e {
            o.insert(k.clone(), val.clone());
        }
    }
    v
}

#[test]
fn contents_request_encodes_stream_as_one_path_component() {
    let h = harness();
    h.mock.push_json(json!({"items": [], "continuation": "abc"}));
    let page = h
        .client
        .contents(
            &StreamId::new("user/1005921515/label/Tech/Caf\u{e9} & more"),
            &ContentsParams {
                count: 500,
                exclude_read: true,
                continuation: Some("tok/+=".into()),
                annotations: true,
                summaries: true,
                manual_tags_only: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(page.continuation.as_deref(), Some("abc"));
    let req = h.mock.last();
    assert_eq!(req.method, Method::Get);
    let path = url::Url::parse(&req.url).unwrap().path().to_string();
    assert_eq!(path, "/reader/api/0/stream/contents/user%2F-%2Flabel%2FTech%2FCaf%C3%A9%20%26%20more");
    assert_eq!(q1(&req.url, "n").as_deref(), Some("100"), "count is capped at 100");
    assert_eq!(q1(&req.url, "xt").as_deref(), Some("user/-/state/com.google/read"));
    assert_eq!(q1(&req.url, "c").as_deref(), Some("tok/+="), "continuation is opaque and round-trips");
    assert_eq!(q1(&req.url, "annotations").as_deref(), Some("1"));
    assert_eq!(q1(&req.url, "summaries").as_deref(), Some("1"));
    assert_eq!(q1(&req.url, "includeAllDirectStreamIds").as_deref(), Some("false"));
    let auth = req.headers.iter().find(|(k, _)| k == "Authorization").unwrap();
    assert_eq!(auth.1, "Bearer acc");
    assert!(req.headers.iter().any(|(k, v)| k == "User-Agent" && v.starts_with("InoInk/")));
}

#[test]
fn contents_items_keep_raw_annotations_summaries_and_unknown_fields() {
    let h = harness();
    h.mock.push_json(json!({"items": [item_json("tag:google.com,2005:reader/item/0000000693c3bc0c", json!({}))]}));
    let page = h.client.contents(&StreamId::new(STATE_ANNOTATED), &ContentsParams { count: 20, ..Default::default() }).unwrap();
    assert!(page.continuation.is_none());
    let a = &page.items[0];
    assert_eq!(a.id, 0x693c3bc0c);
    assert_eq!(a.title, "Windows and Linux devices");
    assert_eq!(a.origin_stream, "feed/http://feeds.arstechnica.com/arstechnica/gadgets");
    assert_eq!(a.canonical_url, "https://arstechnica.com/?p=1755573");
    assert_eq!(a.timestamp_usec, 1618211779000000);
    assert_eq!(a.summary_html, "<p>hello</p>");
    assert!(!a.excerpt, "no excerpt flag is ever invented");
    let raw: serde_json::Value = serde_json::from_str(&a.raw_json).unwrap();
    assert_eq!(raw["annotations"][0]["note"], "Check your firewall!");
    assert_eq!(raw["summaries"][0]["prompt_name"], "One sentence summary");
    assert_eq!(raw["someFutureField"]["keep"], true);
}

#[test]
fn id_forms_in_items_cover_negative_and_high_values() {
    let h = harness();
    h.mock.push_json(json!({"items": [
        item_json("tag:google.com,2005:reader/item/ffffffffffffffff", json!({})),
        item_json("tag:google.com,2005:reader/item/8000000000000000", json!({})),
    ]}));
    let page = h.client.contents(&StreamId::new(STATE_STARRED), &ContentsParams::default()).unwrap();
    assert_eq!(page.items[0].id, u64::MAX);
    assert_eq!(ArticleId(page.items[0].id).to_short(), "-1");
    assert_eq!(ArticleId(page.items[1].id).to_short(), i64::MIN.to_string());
}

#[test]
fn item_ids_request_and_response() {
    let h = harness();
    h.mock.push_json(json!({"items": [], "itemRefs": [
        {"id": "3614359203", "directStreamIds": ["user/1005921515/label/MTB"], "timestampUsec": "1416313130505268"},
        {"id": "-5", "timestampUsec": "1"}
    ], "continuation": "aDRTXr1ek6qT"}));
    let mut p = IdsParams::new(StreamId::new("user/-/state/com.google/starred"));
    p.count = 5000;
    p.only_starred = true;
    p.continuation = Some("prev".into());
    let page = h.client.item_ids(&p).unwrap();
    let req = h.mock.last();
    assert!(req.url.contains("/stream/items/ids?"));
    assert_eq!(q1(&req.url, "s").as_deref(), Some("user/-/state/com.google/starred"));
    assert_eq!(q1(&req.url, "n").as_deref(), Some("1000"));
    assert_eq!(q1(&req.url, "output").as_deref(), Some("json"));
    assert_eq!(q1(&req.url, "it").as_deref(), Some("user/-/state/com.google/starred"));
    assert_eq!(q1(&req.url, "includeAllDirectStreamIds").as_deref(), Some("false"), "string false, not 0");
    assert_eq!(page.refs.len(), 2);
    assert_eq!(page.refs[0].id, ArticleId(3614359203));
    assert_eq!(page.refs[0].direct_stream_ids, vec!["user/-/label/MTB"]);
    assert_eq!(page.refs[1].id, ArticleId((-5i64) as u64));
    assert_eq!(page.continuation.as_deref(), Some("aDRTXr1ek6qT"));
}

#[test]
fn edit_tag_form_repeats_i_uses_short_ids_and_checks_ok() {
    let h = harness();
    h.mock.push_ok();
    h.client.edit_tag(&[ArticleId(12345678), ArticleId(u64::MAX)], Some(&Membership::Read), Some(&Membership::Starred)).unwrap();
    let req = h.mock.last();
    assert_eq!(req.method, Method::Post);
    assert!(req.url.ends_with("/edit-tag"));
    assert_eq!(form_get(&req, "a"), vec!["user/-/state/com.google/read"]);
    assert_eq!(form_get(&req, "r"), vec!["user/-/state/com.google/starred"]);
    assert_eq!(form_get(&req, "i"), vec!["12345678", "-1"]);

    h.mock.push_ok();
    h.client.edit_tag(&[ArticleId(1)], Some(&Membership::Label("my tag/x".into())), None).unwrap();
    assert_eq!(form_get(&h.mock.last(), "a"), vec!["user/-/label/my tag/x"]);

    // Bounds and non-acknowledgement.
    let many: Vec<ArticleId> = (0..=MAX_EDIT_IDS as u64).map(ArticleId).collect();
    assert!(matches!(h.client.edit_tag(&many, Some(&Membership::Read), None), Err(Error::InvalidRequest(_))));
    assert!(h.client.edit_tag(&[], Some(&Membership::Read), None).is_err());
    h.mock.push(Ok(resp(200, "Error=Something")));
    assert!(matches!(h.client.edit_tag(&[ArticleId(1)], Some(&Membership::Read), None), Err(Error::InvalidRequest(_))));
}

#[test]
fn all_system_tags_map_to_documented_ids() {
    assert_eq!(Membership::Read.tag_id(), "user/-/state/com.google/read");
    assert_eq!(Membership::Starred.tag_id(), "user/-/state/com.google/starred");
    assert_eq!(Membership::Like.tag_id(), "user/-/state/com.google/like");
    assert_eq!(Membership::Broadcast.tag_id(), "user/-/state/com.google/broadcast");
}

#[test]
fn subscription_endpoints() {
    let h = harness();
    h.mock.push_json(json!({"subscriptions": [{
        "id": "feed/http://a/rss", "title": "A", "sortid": "0A1B2C3D", "firstitemmsec": "1500000000000000",
        "url": "http://a/rss", "htmlUrl": "http://a/",
        "categories": [{"id": "user/1005921515/label/Tech", "label": "Tech"}]
    }]}));
    let subs = h.client.subscriptions().unwrap();
    assert_eq!(subs[0].folders, vec!["Tech"]);
    assert_eq!(subs[0].first_item_usec, 1500000000000000);

    h.mock.push_ok();
    h.client
        .edit_subscription(&SubscriptionEdit {
            action: SubAction::Edit,
            stream: StreamId::feed("http://a/rss"),
            title: Some("New title".into()),
            add_folder: Some("News".into()),
            remove_folder: Some("Tech".into()),
        })
        .unwrap();
    let r = h.mock.last();
    assert!(r.url.ends_with("/subscription/edit"));
    assert_eq!(form_get(&r, "ac"), vec!["edit"]);
    assert_eq!(form_get(&r, "s"), vec!["feed/http://a/rss"]);
    assert_eq!(form_get(&r, "t"), vec!["New title"]);
    assert_eq!(form_get(&r, "a"), vec!["user/-/label/News"]);
    assert_eq!(form_get(&r, "r"), vec!["user/-/label/Tech"]);

    h.mock.push_ok();
    h.client
        .edit_subscription(&SubscriptionEdit {
            action: SubAction::Unsubscribe,
            stream: StreamId::feed("http://a/rss"),
            title: None,
            add_folder: None,
            remove_folder: None,
        })
        .unwrap();
    assert_eq!(form_get(&h.mock.last(), "ac"), vec!["unsubscribe"]);
    assert!(form_get(&h.mock.last(), "t").is_empty());

    h.mock.push_json(json!({"query": "http://b/", "numResults": 1, "streamId": "feed/http://b/rss"}));
    let q = h.client.quickadd("http://b/").unwrap();
    assert_eq!(q.num_results, 1);
    assert_eq!(q.stream_id.as_deref(), Some("feed/http://b/rss"));
    assert_eq!(form_get(&h.mock.last(), "quickadd"), vec!["http://b/"]);
}

#[test]
fn tag_endpoints_and_unread_counts() {
    let h = harness();
    h.mock.push_json(json!({"tags": [
        {"id": "user/1005921515/state/com.google/starred", "sortid": "FFFFFFFF"},
        {"id": "user/1005921515/label/Tech", "sortid": "01", "type": "folder", "unread_count": 3},
        {"id": "user/1005921515/label/ToRead", "type": "tag"}
    ]}));
    let tags = h.client.tags().unwrap();
    assert_eq!(tags[1].id, "user/-/label/Tech");
    assert_eq!(tags[1].kind, "folder");
    assert_eq!(q1(&h.mock.last().url, "types").as_deref(), Some("1"));
    assert_eq!(q1(&h.mock.last().url, "counts").as_deref(), Some("1"));

    h.mock.push_ok();
    h.client.rename_tag(&StreamId::label("Tech"), "Technology").unwrap();
    assert_eq!(form_get(&h.mock.last(), "s"), vec!["user/-/label/Tech"]);
    assert_eq!(form_get(&h.mock.last(), "dest"), vec!["user/-/label/Technology"]);
    h.mock.push_ok();
    h.client.disable_tag(&StreamId::label("Old")).unwrap();
    assert!(h.mock.last().url.ends_with("/disable-tag"));

    h.mock.push_json(json!({"max": 1000, "unreadcounts": [
        {"id": "user/1005921515/state/com.google/reading-list", "count": 1000, "newestItemTimestampUsec": "1416313130505268"},
        {"id": "feed/http://a", "count": 12, "newestItemTimestampUsec": "5"}
    ]}));
    let c = h.client.unread_counts().unwrap();
    assert_eq!(c.display(1000), "1000+");
    assert_eq!(c.display(12), "12");
    assert_eq!(c.counts[0].0, "user/-/state/com.google/reading-list");
}

#[test]
fn stream_preferences_and_ordering() {
    let h = harness();
    h.mock.push_json(json!({"streamprefs": {
        "user/1005921515/state/com.google/root": [{"id": "subscription-ordering", "value": "0A1B2C3D4E5F6A7B"}],
        "user/1005921515/label/Tech": [{"id": "other", "value": "x"}]
    }}));
    let prefs = h.client.stream_preferences().unwrap();
    assert_eq!(prefs["user/-/state/com.google/root"], "0A1B2C3D4E5F6A7B");
    assert_eq!(split_sort_ids("0A1B2C3D4E5F6A7B"), vec!["0A1B2C3D", "4E5F6A7B"]);

    h.mock.push_ok();
    h.client.set_stream_order(&StreamId::new(STATE_ROOT), &["0A1B2C3D".into(), "4E5F6A7B".into()]).unwrap();
    let r = h.mock.last();
    assert_eq!(form_get(&r, "k"), vec!["subscription-ordering"]);
    assert_eq!(form_get(&r, "v"), vec!["0A1B2C3D4E5F6A7B"]);

    let before = h.mock.count();
    for bad in ["short", "0A1B2C3Z", "0A1B2C3D4"] {
        assert!(matches!(h.client.set_stream_order(&StreamId::new(STATE_ROOT), &[bad.into()]), Err(Error::InvalidRequest(_))), "{bad}");
    }
    assert_eq!(h.mock.count(), before, "invalid preferences never reach the network");
}

#[test]
fn mark_all_as_read_sends_cutoff() {
    let h = harness();
    h.mock.push_ok();
    h.client.mark_all_read(&StreamId::feed("http://a"), 1618211779).unwrap();
    let r = h.mock.last();
    assert!(r.url.ends_with("/mark-all-as-read"));
    assert_eq!(form_get(&r, "ts"), vec!["1618211779"]);
    assert_eq!(form_get(&r, "s"), vec!["feed/http://a"]);
}

#[test]
fn active_search_create_and_delete() {
    let h = harness();
    h.mock.push_json(json!({"id": "user/1005921515/search/abc123", "title": "rust"}));
    let created = h
        .client
        .create_active_search(&ActiveSearchSpec {
            term: "rust lang".into(),
            filter_type: FilterType::Folder,
            filter_id: Some("user/-/label/Tech".into()),
            article_ids: vec![ArticleId(7)],
            search_filters: Some("titles".into()),
            search_match: Some("full_phrase".into()),
            search_feed_popularity: None,
        })
        .unwrap();
    assert_eq!(created.id, "user/-/search/abc123");
    let r = h.mock.last();
    assert!(r.url.ends_with("/active_search/create"));
    let Some(Body::Json(j)) = &r.body else { panic!("json body expected") };
    let v: serde_json::Value = serde_json::from_str(j).unwrap();
    assert_eq!(v["term"], "rust lang");
    assert_eq!(v["filter_type"], "folder");
    assert_eq!(v["filter_id"], "user/-/label/Tech");
    assert_eq!(v["article_ids"][0], "7");
    assert_eq!(v["search_filters"], "titles");
    assert_eq!(v["search_match"], "full_phrase");

    // Validation failure arrives as HTTP 202 with a description.
    h.mock.push(Ok(resp(202, &json!({"accountLimit": 30, "userOutput": "You reached your limit"}).to_string())));
    match h.client.create_active_search(&ActiveSearchSpec {
        term: "x".into(),
        filter_type: FilterType::Public,
        filter_id: None,
        article_ids: vec![],
        search_filters: None,
        search_match: None,
        search_feed_popularity: Some("popular_sites".into()),
    }) {
        Err(Error::InvalidRequest(m)) => assert!(m.contains("limit")),
        other => panic!("{other:?}"),
    }
    assert!(h
        .client
        .create_active_search(&ActiveSearchSpec {
            term: "x".into(),
            filter_type: FilterType::Folder,
            filter_id: None,
            article_ids: vec![],
            search_filters: None,
            search_match: None,
            search_feed_popularity: None
        })
        .is_err());

    // Delete is the documented GET.
    h.mock.push(Ok(resp(200, "ok")));
    h.client.delete_active_search(&StreamId::new("user/1005921515/search/abc123")).unwrap();
    let r = h.mock.last();
    assert_eq!(r.method, Method::Get);
    assert_eq!(q1(&r.url, "id").as_deref(), Some("user/-/search/abc123"));
}

#[test]
fn user_info_and_errors() {
    let h = harness();
    h.mock.push_json(json!({"userId": "1005921515", "userName": "Y", "userEmail": "y@example.org", "isBloggerUser": false}));
    assert_eq!(h.client.user_info().unwrap().user_id, "1005921515");
    h.mock.push(Ok(resp(403, "")));
    assert!(matches!(h.client.user_info(), Err(Error::PermissionDenied(_))));
    h.mock.push(Ok(resp(400, "Error=Missing parameter")));
    assert!(matches!(h.client.user_info(), Err(Error::InvalidRequest(_))));
    h.mock.push(Ok(resp(503, "")));
    assert!(matches!(h.client.user_info(), Err(Error::Retryable(_))));
    h.mock.push(Err(Error::Offline));
    assert!(matches!(h.client.user_info(), Err(Error::Offline)));
}

#[test]
fn malformed_and_oversized_responses_are_rejected() {
    let h = harness();
    h.mock.push(Ok(resp(200, "<html>captive portal</html>")));
    assert!(matches!(h.client.user_info(), Err(Error::Data(_))));
    h.mock.push_json(json!({"items": "nope"}));
    assert!(matches!(h.client.contents(&StreamId::new(STATE_STARRED), &ContentsParams::default()), Err(Error::Data(_))));
    h.mock.push_json(json!({"items": [{"title": "no id"}]}));
    assert!(matches!(h.client.contents(&StreamId::new(STATE_STARRED), &ContentsParams::default()), Err(Error::Data(_))));
    let big = HttpResponse { status: 200, headers: vec![], body: vec![b' '; MAX_BODY + 1] };
    h.mock.push(Ok(big));
    assert!(matches!(h.client.user_info(), Err(Error::Data(_))));
}

#[test]
fn unknown_outcome_on_get_becomes_retryable_but_post_is_surfaced() {
    let h = harness();
    h.mock.push(Err(Error::UnknownOutcome("lost".into())));
    assert!(matches!(h.client.user_info(), Err(Error::Retryable(_))));
    h.mock.push(Err(Error::UnknownOutcome("lost".into())));
    assert!(matches!(h.client.quickadd("http://x"), Err(Error::UnknownOutcome(_))));
}

#[test]
fn expired_token_refreshes_once_before_the_request_and_persists_rotation() {
    let h = harness();
    h.clock.advance(3600); // access token now expired
    h.mock.push_json(json!({"access_token": "acc2", "refresh_token": "ref2", "expires_in": 3600, "scope": "read write"}));
    h.mock.push_json(json!({"userId": "1"}));
    h.client.user_info().unwrap();
    let reqs = h.mock.all();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].url, "https://example.test/oauth2/token");
    assert_eq!(form_get(&reqs[0], "grant_type"), vec!["refresh_token"]);
    assert_eq!(form_get(&reqs[0], "refresh_token"), vec!["ref"]);
    assert_eq!(form_get(&reqs[0], "client_secret"), vec!["secret"]);
    assert!(reqs[1].headers.iter().any(|(k, v)| k == "Authorization" && v == "Bearer acc2"));
    let saved = h.tokens.saves.lock().unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].refresh_token, "ref2", "rotated refresh token is persisted");
}

#[test]
fn a_401_triggers_exactly_one_refresh_and_retry() {
    let h = harness();
    h.mock.push(Ok(resp(401, "")));
    h.mock.push_json(json!({"access_token": "acc2", "refresh_token": "ref", "expires_in": 3600}));
    h.mock.push_json(json!({"userId": "1"}));
    h.client.user_info().unwrap();
    assert_eq!(h.mock.count(), 3);

    // A second 401 after refreshing is final.
    h.mock.push(Ok(resp(401, "")));
    h.mock.push_json(json!({"access_token": "acc3", "refresh_token": "ref", "expires_in": 3600}));
    h.mock.push(Ok(resp(401, "")));
    assert!(matches!(h.client.user_info(), Err(Error::NeedsAuthorization)));
    assert_eq!(h.mock.count(), 6);
}

#[test]
fn revoked_refresh_token_asks_to_sign_in_again() {
    let h = harness();
    h.clock.advance(7200);
    h.mock.push(Ok(resp(400, r#"{"error":"invalid_grant"}"#)));
    assert!(matches!(h.client.user_info(), Err(Error::NeedsAuthorization)));
    assert_eq!(h.mock.count(), 1, "no API request is made without a valid token");
}

#[test]
fn no_tokens_means_needs_authorization_without_network() {
    let mock = Mock::new();
    let store = std::sync::Arc::new(MemTokens::default());
    let cfg = AppConfig::new("c", "s", "http://localhost/cb");
    let client = ApiClient::new(cfg, mock.clone(), FixedClock::new(0), store).unwrap();
    assert!(matches!(client.user_info(), Err(Error::NeedsAuthorization)));
    assert_eq!(mock.count(), 0);
}

#[test]
fn oauth_consent_url_and_callback_validation() {
    let cfg = AppConfig::new("my id", "s3cret", "http://localhost:8080/cb");
    let url = auth::authorize_url(&cfg, "st&te", "read write");
    assert!(url.starts_with("https://www.inoreader.com/oauth2/auth?"));
    assert_eq!(q1(&url, "client_id").as_deref(), Some("my id"));
    assert_eq!(q1(&url, "redirect_uri").as_deref(), Some("http://localhost:8080/cb"));
    assert_eq!(q1(&url, "response_type").as_deref(), Some("code"));
    assert_eq!(q1(&url, "scope").as_deref(), Some("read write"));
    assert_eq!(q1(&url, "state").as_deref(), Some("st&te"));
    assert!(!url.contains("s3cret"), "the client secret never appears in the consent URL");

    assert_eq!(auth::parse_callback("http://localhost:8080/cb?code=XYZ&state=abc", "abc").unwrap(), "XYZ");
    assert!(auth::parse_callback("http://localhost:8080/cb?code=XYZ&state=evil", "abc").is_err());
    assert!(auth::parse_callback("http://localhost:8080/cb?code=XYZ", "abc").is_err());
    assert!(auth::parse_callback("http://localhost:8080/cb?error=access_denied&state=abc", "abc").is_err());
}

#[test]
fn code_exchange_posts_documented_form() {
    let mock = Mock::new();
    let clock = FixedClock::new(100);
    let cfg = AppConfig::new("cid", "s3cr3t-value", "http://localhost:8080/cb");
    mock.push_json(json!({"access_token": "A", "token_type": "Bearer", "expires_in": 3600, "refresh_token": "R", "scope": "read write"}));
    let t = auth::exchange_code(&cfg, mock.as_ref(), clock.as_ref(), "CODE").unwrap();
    assert_eq!(t.access_token, "A");
    assert_eq!(t.expires_at, 3700);
    let r = mock.last();
    assert_eq!(r.url, "https://www.inoreader.com/oauth2/token");
    assert_eq!(form_get(&r, "grant_type"), vec!["authorization_code"]);
    assert_eq!(form_get(&r, "code"), vec!["CODE"]);
    assert_eq!(form_get(&r, "client_id"), vec!["cid"]);
    assert!(format!("{t:?}").contains("<redacted>") && !format!("{t:?}").contains("\"A\""));
    assert!(!format!("{cfg:?}").contains("s3cr3t-value"));
}

#[test]
fn quota_headers_drive_pause_and_reserve() {
    let h = harness();
    fn hdrs(u2: &str) -> Vec<(&str, &str)> {
        vec![
            ("x-reader-zone1-limit", "100"),
            ("x-reader-zone1-usage", "10"),
            ("x-reader-zone2-limit", "100"),
            ("x-reader-zone2-usage", u2),
            ("x-reader-limits-reset-after", "600"),
        ]
    }
    h.mock.push(Ok(resp_h(200, "OK", &hdrs("91"))));
    h.client.edit_tag(&[ArticleId(1)], Some(&Membership::Read), None).unwrap();
    let q = h.client.quota();
    let r = q.remaining(Zone::Two);
    assert_eq!((r.remaining, r.limit, r.estimated), (9, 100, false));
    assert_eq!(q.reserve(Zone::Two), 10);

    // Background work stops at the reserve; explicit actions still go through.
    let bg = h.client.with_priority(Priority::Background);
    assert!(matches!(bg.edit_tag(&[ArticleId(1)], Some(&Membership::Read), None), Err(Error::QuotaPaused { reset_after_secs: 600 })));
    h.mock.push(Ok(resp_h(200, "OK", &hdrs("92"))));
    h.client.with_priority(Priority::Explicit).edit_tag(&[ArticleId(1)], Some(&Membership::Read), None).unwrap();
    // Zone 1 is unaffected by zone 2's exhaustion.
    h.mock.push_json(json!({"userId": "1"}));
    bg.user_info().unwrap();

    // After the reset time, usage is forgotten.
    h.clock.advance(601);
    h.mock.push_ok();
    bg.edit_tag(&[ArticleId(1)], Some(&Membership::Read), None).unwrap();
}

#[test]
fn http_429_pauses_until_reset_and_survives_restart() {
    let h = harness();
    h.mock.push(Ok(resp_h(429, "", &[("x-reader-limits-reset-after", "900")])));
    let first = h.client.user_info();
    assert!(matches!(first, Err(Error::QuotaPaused { reset_after_secs: 900 })));
    let n = h.mock.count();
    assert!(matches!(h.client.with_priority(Priority::Explicit).user_info(), Err(Error::QuotaPaused { .. })));
    assert_eq!(h.mock.count(), n, "no request while paused, even an explicit one");

    // Persist, then restore into a fresh client: still paused.
    let saved = h.client.quota_json();
    let h2 = harness();
    h2.client.restore_quota(&saved);
    assert!(matches!(h2.client.user_info(), Err(Error::QuotaPaused { .. })));
    assert_eq!(h2.mock.count(), 0);
    h2.clock.advance(901);
    h2.mock.push_json(json!({"userId": "1"}));
    h2.client.user_info().unwrap();
}

#[test]
fn quota_falls_back_to_a_labelled_estimate_without_headers() {
    let h = harness();
    h.mock.push_json(json!({"userId": "1"}));
    h.client.user_info().unwrap();
    let r = h.client.quota().remaining(Zone::One);
    assert!(r.estimated);
    assert_eq!((r.limit, r.remaining), (100, 99));
}
