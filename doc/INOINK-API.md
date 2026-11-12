# Inoreader API capability matrix

Audited against the public developer documentation (https://www.inoreader.com/developers/)
at implementation start. Base URL `https://www.inoreader.com/reader/api/0` unless stated.
"Live" is the status against a real account; nothing is live-verified until stated.
Module paths are in `crates/inoink/src`.

| Capability | Method and parameters | Interface | Module function | Test | Live | Limitations |
|---|---|---|---|---|---|---|
| OAuth consent | `GET https://www.inoreader.com/oauth2/auth` client_id, redirect_uri, response_type=code, scope, state | `inoink-cli authorize` | `api::auth::authorize_url` | `api_contract` | not run | Documented flow needs a client secret; no documented PKCE/public client |
| OAuth token / refresh | `POST /oauth2/token` form: code or refresh_token, client_id, client_secret, grant_type | `inoink-cli authorize`; automatic on the device | `api::auth::{exchange_code,refresh}` | `api_contract` | not run | Secret is user-owned and stored on the device |
| User info | `GET /user-info` | account id for the database | `ApiClient::user_info` | `api_contract` | not run | |
| Subscription list | `GET /subscription/list` | drawer: Feeds; Manage | `ApiClient::subscriptions` | `api_contract` | not run | `firstitemmsec` is microseconds |
| Add feed | `POST /subscription/quickadd` quickadd | Manage > Follow a feed | `ApiClient::quickadd` | `api_contract`, `sync_faults` | not run | numResults=1 also for existing subscription; not idempotent, reconciled via list |
| Edit / unsubscribe | `POST /subscription/edit` ac=edit\| Manage > Feeds |subscribe\|unsubscribe, s, t, a, r | `ApiClient::edit_subscription` | `api_contract` | not run | |
| Unread counts | `GET /unread-count` | first request of every sync: change signal for listings and the organization | `ApiClient::unread_counts` | `api_contract`, `sync_faults` | PASS (device data 2026-11-12: lists every feed and folder, reading list with newest timestamp, starred total) | Capped counts say nothing; equal counts can hide a swap (resolved by the 6-hour re-listing) |
| Folder/tag list | `GET /tag/list` types, counts, team_assets | drawer: Folders, Tags, Monitoring | `ApiClient::tags` | `api_contract` | not run | No documented way to create an empty folder/tag |
| Rename tag | `POST /rename-tag` s, dest | Manage > Folders | `ApiClient::rename_tag` | `api_contract` | not run | Not idempotent |
| Delete tag | `POST /disable-tag` s | Manage > Folders | `ApiClient::disable_tag` | `api_contract` | not run | |
| Stream contents | `GET /stream/contents/{stream}` n<=100, r, ot, xt, it, c, annotations, summaries, includeAllDirectStreamIds | list, history paging, Load more at the end of a list | `ApiClient::contents` | `api_contract` | not run | Content may be an excerpt; no completeness flag; `ot` with r=o clamps to one month; also read from the Annotated stream to refresh annotations |
| Item IDs | `GET /stream/items/ids` n<=1000, r, ot, xt, it, c, s, output=json, includeAllDirectStreamIds | internal reconciliation | `ApiClient::item_ids` | `api_contract`, `sync_faults` | not run | No consistent snapshot token |
| Atom stream | `GET /reader/atom` | none | not implemented | - | n/a | Documented compatibility alternative to the JSON contents endpoint; InoInk uses JSON only, as the docs advise |
| Edit tag | `POST /edit-tag` a, r, repeated i (read, starred, broadcast, like, user label) | Star, Done, Mark read, Like, Broadcast, Tags | `ApiClient::edit_tag` | `api_contract`, `store_intent`, `sync_faults` | not run | Marking older than `firstitemmsec` unread is silently ignored |
| Stream preferences | `GET /preference/stream/list` | stored; no control yet | `ApiClient::stream_preferences` | `api_contract` | not run | |
| Set ordering | `POST /preference/stream/set` s, k=subscription-ordering, v | none yet (sync layer only) | `ApiClient::set_stream_order` | `api_contract` | not run | Only `subscription-ordering` accepted |
| Mark all as read | `POST /mark-all-as-read` s, ts | none by design (see limitation) | `ApiClient::mark_all_read` | `api_contract` | not run | Implemented but not used by UI bulk actions: a timestamp cutoff cannot guarantee scope. UI bulk enumerates frozen IDs and uses edit-tag |
| Active Search create | `POST /active_search/create` JSON term, filter_type, filter_id, article_ids, search_filters, search_match, search_feed_popularity | Manage > Monitoring | `ApiClient::create_active_search` | `api_contract` | not run | Validation failures can be HTTP 202; Pro limit 30; not idempotent |
| Active Search delete | `GET /active_search/delete` id | Manage > Monitoring (confirmed) | `ApiClient::delete_active_search` | `api_contract` | not run | Destructive GET per docs; confirmed in UI, never retried blindly |
| Saved web pages / annotated / like / broadcast streams | stream IDs `user/-/state/com.google/{saved-web-pages,annotated,like,broadcast}` | `Collection` | `api_contract` | not run | Saved-pages content equals what the API returns; not guaranteed identical to the website's views |
| Rate limiting | headers `X-Reader-Zone{1,2}-{Limit,Usage}`, `X-Reader-Limits-Reset-After`; 429 | Settings > Inoreader API budget; Main menu > Status | `api::quota` | `sync_faults` | PASS (2000 per zone reported live) | Limits depend on the plan; until a header is seen, 100 per zone is assumed and labelled estimated |

## Deliberately not called (no documented request contract)

`/token`, `/save-user-pref`, `/preference/list`, `/mark-all-as-read-undo`,
`/stream/items/contents` appear only in the rate-limit table.

## Genuine API limitations (no documented endpoint)

- Interactive account-wide search: search is local, over downloaded articles. Active Search
  creates persistent monitoring feeds and is not a transient query.
- Creating/removing a saved web page from an arbitrary URL.
- Annotation editing: no documented endpoint. The website uses private xajax calls
  (`new_annotation`, `edit_annotation`, `delete_annotation`) with session cookies; InoInk does not
  call them. Offsets are UTF-16 code units of the rendered article text (`Range.toString()`),
  verified in the website's `annotations.js`; InoInk anchors by quote instead.
- Full-content extraction: optional unauthenticated original-page extraction is an InoInk
  local feature, not an Inoreader API. Discussion threads for Hacker News (hn.algolia.com) and
  Lobsters (public JSON) are fetched the same way, without credentials.
- Rule management; web-feed / change-tracking management.
- Reading-position sync: positions are local only.
- Concurrent offline edits: there is no revision history, so concurrent changes are
  inherently ambiguous (see `doc/INOINK-SYNC.md`).
