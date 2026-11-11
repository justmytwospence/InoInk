-- Articles, observed server state, pending local intent, undo, bulk scopes, positions.
CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE articles (
    account TEXT NOT NULL,
    id INTEGER NOT NULL,                 -- 64-bit article id, stored as signed
    title TEXT NOT NULL DEFAULT '',
    author TEXT NOT NULL DEFAULT '',
    origin_stream TEXT NOT NULL DEFAULT '',
    origin_title TEXT NOT NULL DEFAULT '',
    canonical_url TEXT NOT NULL DEFAULT '',
    published INTEGER NOT NULL DEFAULT 0,
    timestamp_usec INTEGER NOT NULL DEFAULT 0,
    summary_html TEXT NOT NULL DEFAULT '',
    content_hash TEXT NOT NULL DEFAULT '',
    raw_json TEXT NOT NULL DEFAULT '{}',  -- original API object, immutable once stored
    excerpt INTEGER NOT NULL DEFAULT 0,   -- 1 only when evidence says the content is an excerpt
    download_state TEXT NOT NULL DEFAULT 'not_downloaded',
    pinned INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account, id)
) WITHOUT ROWID;
CREATE INDEX articles_order ON articles (account, timestamp_usec DESC, id DESC);
CREATE INDEX articles_origin ON articles (account, origin_stream, timestamp_usec DESC);

-- Streams an article was directly listed under (labels, saved pages, annotated, ...).
CREATE TABLE article_streams (
    account TEXT NOT NULL,
    article INTEGER NOT NULL,
    stream TEXT NOT NULL,
    PRIMARY KEY (account, article, stream)
) WITHOUT ROWID;
CREATE INDEX article_streams_by_stream ON article_streams (account, stream);

-- Last-known server state per article membership. `seq` orders it against sync runs.
CREATE TABLE remote_state (
    account TEXT NOT NULL,
    article INTEGER NOT NULL,
    key TEXT NOT NULL,
    present INTEGER NOT NULL,
    seq INTEGER NOT NULL,
    PRIMARY KEY (account, article, key)
) WITHOUT ROWID;

-- Pending local intent. `generation` is the latest desired value; `sent_generation`
-- (with `sent_desired`) is what is currently being sent to the server, if anything.
CREATE TABLE intents (
    account TEXT NOT NULL,
    article INTEGER NOT NULL,
    key TEXT NOT NULL,
    desired INTEGER NOT NULL,
    generation INTEGER NOT NULL,
    sent_generation INTEGER,
    sent_desired INTEGER,
    updated INTEGER NOT NULL,
    PRIMARY KEY (account, article, key)
) WITHOUT ROWID;

CREATE TABLE undos (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account TEXT NOT NULL,
    article INTEGER NOT NULL,
    prev_read INTEGER NOT NULL,
    prev_starred INTEGER NOT NULL,
    created INTEGER NOT NULL,
    used INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE bulk_scopes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account TEXT NOT NULL,
    created INTEGER NOT NULL
);
CREATE TABLE bulk_items (
    scope INTEGER NOT NULL REFERENCES bulk_scopes(id) ON DELETE CASCADE,
    article INTEGER NOT NULL,
    PRIMARY KEY (scope, article)
) WITHOUT ROWID;

CREATE TABLE positions (
    account TEXT NOT NULL,
    article INTEGER NOT NULL,
    content_hash TEXT NOT NULL,
    offset INTEGER NOT NULL,
    anchor TEXT NOT NULL DEFAULT '',
    font_size REAL,
    updated INTEGER NOT NULL,
    PRIMARY KEY (account, article)
) WITHOUT ROWID;

-- Reconciliation runs: a paged listing of one membership, compared against remote_state.
CREATE TABLE sync_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account TEXT NOT NULL,
    key TEXT NOT NULL,              -- membership key being reconciled
    listed_value INTEGER NOT NULL,  -- value implied for ids that appear in the listing
    start_seq INTEGER NOT NULL,
    complete INTEGER NOT NULL DEFAULT 0,
    abandoned INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE sync_seen (
    run INTEGER NOT NULL REFERENCES sync_runs(id) ON DELETE CASCADE,
    article INTEGER NOT NULL,
    PRIMARY KEY (run, article)
) WITHOUT ROWID;
CREATE TABLE sync_tokens (
    run INTEGER NOT NULL REFERENCES sync_runs(id) ON DELETE CASCADE,
    token TEXT NOT NULL,
    PRIMARY KEY (run, token)
) WITHOUT ROWID;
