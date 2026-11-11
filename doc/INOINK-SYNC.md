# How synchronization works

## Principles

- **Inoreader is the source of truth.** The local database holds a copy and your pending intent.
- **Save first.** Every change (star, read, tag, folder, subscription) is committed to SQLite
  in one transaction before the interface reports it; the screen updates from that commit.
- **Server state and pending intent are separate.** Each article has a last-known server value per
  membership (read, starred, like, broadcast, each tag) and, if you changed it, a pending intent.
  The interface shows the intent when there is one, else the server value.
- **Acknowledged means the server said OK.** An intent is cleared only by a successful response,
  and only the generation that was sent: a newer value you set while the request was in flight
  stays pending.
- **Redundant changes collapse.** Starring and unstarring while offline sends nothing if the final
  value equals the server's. Done is one request (add read, remove star).
- **Tags are independent.** Each tag is its own membership; adding one never rewrites the others.

## Reconciliation

Starred and unread are reconciled with Inoreader's lightweight ID listings (`stream/items/ids`,
1000 per page), then missing articles are fetched with `stream/contents`. Old articles starred
elsewhere are found because the listing does not depend on publication time; they are downloaded
for offline reading regardless of age.

An article is only marked as having *left* a collection if a **complete** listing omits it **and a
second complete listing agrees**. A failed, partial, looping or page-capped listing never removes
anything. Leaving Starred does not delete the article or remove it from All articles or a folder.
A snapshot that began before one of your acknowledged writes cannot overwrite it.

## Conflicts

Pending local intent wins until acknowledged; after that, anything the server later reports wins.
Inoreader offers no revision history or conditional writes, so two devices changing the same item
while one is offline cannot be ordered: the last write to reach the server wins and the other
device observes it at its next sync. This ambiguity is inherent to the API.

Writes that set a state (read, star, like, broadcast, tags, folder membership, ordering) are safe
to send again, so a request that succeeded but lost its response is simply sent again. Writes
that are not safe to repeat (follow a feed, rename or delete a folder, create a monitoring
search) become "outcome unknown" and are resolved by reading the subscription and tag lists
before anything is sent again; a monitoring search whose outcome cannot be established is not
retried and is reported.

Inoreader silently ignores marking an article unread when it is older than its feed's first stored
item. InoInk checks the boundary, drops that change, says so, and shows the server's state.

## Quota and scheduling

The server's `X-Reader-Zone{1,2}-{Limit,Usage}` and reset headers are tracked per zone and persist
across restarts; until a header is seen a conservative estimate of 100 per zone is used and
labelled as an estimate. Automatic syncs leave a reserve (at least 5, or 10%) for requests you
make. On HTTP 429, all requests pause until the reset and the status line says so; pending work is
kept. Automatic metadata refresh and reconciliation run at most every 15 minutes; writes are
debounced two seconds (ten at most). There is no timer: sync starts on launch, a wake or network
event, a committed change, or when you ask, so the device is never kept awake.

## Offline content

Original API HTML is kept; a sanitized XHTML version and its images are derived from it.
Images download two at a time (16 MiB and 32 megapixel limits, redirects and private addresses
refused, GIF/WebP converted to PNG, large images scaled). Files are written atomically and
recovered after interruption. Starred articles and pinned folders are always kept; recent unread
(30 days) fill a 512 MB budget, oldest first; evicted articles are shown as not downloaded.
The content is what the feed provides: Inoreader's API exposes no flag saying whether it is an
excerpt, so InoInk says "Feed content" and offers **Load original page**, a best-effort extraction
from the public page that cannot recover paywalled or login-only text.
