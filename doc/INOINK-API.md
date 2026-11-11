# Inoreader API capability matrix

Audited against the public developer documentation (https://www.inoreader.com/developers/)
at implementation start. Base URL `https://www.inoreader.com/reader/api/0` unless stated.
"Live" is the status against a real account; nothing is live-verified until stated.
Module paths are in `crates/inoink/src`.

| Capability | Method and parameters | Module function | Test | Live | Limitations |
|---|---|---|---|---|---|
| OAuth consent | `GET https://www.inoreader.com/oauth2/auth` client_id, redirect_uri, response_type=code, scope, state | `api::auth::authorize_url` | `api_contract` | not run | Documented flow needs a client secret; no documented PKCE/public client |
| OAuth token / refresh | `POST /oauth2/token` form: code or refresh_token, client_id, client_secret, grant_type | `api::auth::{exchange_code,refresh}` | `api_contract` | not run | Secret is user-owned and stored on the device |
| User info | `GET /user-info` | `ApiClient::user_info` | `api_contract` | not run | |
| Subscription list | `GET /subscription/list` | `ApiClient::subscriptions` | `api_contract` | not run | `firstitemmsec` is microseconds |
| Add feed | `POST /subscription/quickadd` quickadd | `ApiClient::quickadd` | `api_contract`, `sync_faults` | not run | numResults=1 also for existing subscription; not idempotent, reconciled via list |
| Edit / unsubscribe | `POST /subscription/edit` ac=edit\|subscribe\|unsubscribe, s, t, a, r | `ApiClient::edit_subscription` | `api_contract` | not run | |
| Unread counts | `GET /unread-count` | `ApiClient::unread_counts` | `api_contract` | not run | Capped counts shown as `N+` |
| Folder/tag list | `GET /tag/list` types, counts, team_assets | `ApiClient::tags` | `api_contract` | not run | No documented way to create an empty folder/tag |
| Rename tag | `POST /rename-tag` s, dest | `ApiClient::rename_tag` | `api_contract` | not run | Not idempotent |
| Delete tag | `POST /disable-tag` s | `ApiClient::disable_tag` | `api_contract` | not run | |
| Stream contents | `GET /stream/contents/{stream}` n<=100, r, ot, xt, it, c, annotations, summaries, includeAllDirectStreamIds | `ApiClient::contents` | `api_contract` | not run | Content may be an excerpt; no completeness flag; `ot` with r=o clamps to one month |
| Item IDs | `GET /stream/items/ids` n<=1000, r, ot, xt, it, c, s, output=json, includeAllDirectStreamIds | `ApiClient::item_ids` | `api_contract`, `sync_faults` | not run | No consistent snapshot token |
| Atom stream | `GET /reader/atom` | parse fixture only | `api_contract` | not run | Alternate representation of contents; not used for sync |
| Edit tag | `POST /edit-tag` a, r, repeated i (read, starred, broadcast, like, user label) | `ApiClient::edit_tag` | `api_contract`, `store_intent`, `sync_faults` | not run | Marking older than `firstitemmsec` unread is silently ignored |
| Stream preferences | `GET /preference/stream/list` | `ApiClient::stream_preferences` | `api_contract` | not run | |
| Set ordering | `POST /preference/stream/set` s, k=subscription-ordering, v | `ApiClient::set_stream_order` | `api_contract` | not run | Only `subscription-ordering` accepted |
| Mark all as read | `POST /mark-all-as-read` s, ts | `ApiClient::mark_all_read` | `api_contract` | not run | Implemented but not used by UI bulk actions: a timestamp cutoff cannot guarantee scope. UI bulk enumerates frozen IDs and uses edit-tag |
| Active Search create | `POST /active_search/create` JSON term, filter_type, filter_id, article_ids, search_filters, search_match, search_feed_popularity | `ApiClient::create_active_search` | `api_contract` | not run | Validation failures can be HTTP 202; Pro limit 30; not idempotent |
| Active Search delete | `GET /active_search/delete` id | `ApiClient::delete_active_search` | `api_contract` | not run | Destructive GET per docs; confirmed in UI, never retried blindly |
| Saved web pages / annotated / like / broadcast streams | stream IDs `user/-/state/com.google/{saved-web-pages,annotated,like,broadcast}` | `Collection` | `api_contract` | not run | Saved-pages content equals what the API returns; not guaranteed identical to the website's views |
| Rate limiting | headers `X-Reader-Zone{1,2}-{Limit,Usage}`, `X-Reader-Limits-Reset-After`; 429 | `api::quota` | `sync_faults` | not run | Defaults 100/zone/day on Pro; fallback is labelled estimated |

## Deliberately not called (no documented request contract)

`/token`, `/save-user-pref`, `/preference/list`, `/mark-all-as-read-undo`,
`/stream/items/contents` appear only in the rate-limit table.

## Genuine API limitations (no documented endpoint)

- Interactive account-wide search: search is local, over downloaded articles. Active Search
  creates persistent monitoring feeds and is not a transient query.
- Creating/removing a saved web page from an arbitrary URL.
- Annotation editing: existing annotations and summaries are displayed read-only; offset
  conventions are not specified.
- Full-content extraction: optional unauthenticated original-page extraction is an InoInk
  local feature, not an Inoreader API.
- Rule management; web-feed / change-tracking management.
- Reading-position sync: positions are local only.
- Concurrent offline edits: there is no revision history, so concurrent changes are
  inherently ambiguous (see `doc/INOINK-SYNC.md`).
