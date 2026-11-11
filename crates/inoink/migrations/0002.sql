-- Organization, counts, content cache bookkeeping and full-text search.
CREATE TABLE subscriptions (
    account TEXT NOT NULL,
    id TEXT NOT NULL,                -- feed stream id
    title TEXT NOT NULL DEFAULT '',
    url TEXT NOT NULL DEFAULT '',
    html_url TEXT NOT NULL DEFAULT '',
    sort_id TEXT NOT NULL DEFAULT '',
    first_item_usec INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account, id)
) WITHOUT ROWID;

CREATE TABLE subscription_folders (
    account TEXT NOT NULL,
    feed TEXT NOT NULL,
    folder TEXT NOT NULL,            -- bare folder name
    PRIMARY KEY (account, feed, folder)
) WITHOUT ROWID;

CREATE TABLE tags (
    account TEXT NOT NULL,
    id TEXT NOT NULL,                -- full stream id
    kind TEXT NOT NULL DEFAULT 'tag',-- tag | folder | active_search
    sort_id TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (account, id)
) WITHOUT ROWID;

CREATE TABLE unread_counts (
    account TEXT NOT NULL,
    stream TEXT NOT NULL,
    count INTEGER NOT NULL,
    newest_usec INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account, stream)
) WITHOUT ROWID;

CREATE TABLE stream_order (
    account TEXT NOT NULL,
    stream TEXT NOT NULL,
    ordering TEXT NOT NULL,
    PRIMARY KEY (account, stream)
) WITHOUT ROWID;

-- Subscription/folder/tag operations that are not per-article state setters.
CREATE TABLE org_ops (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account TEXT NOT NULL,
    kind TEXT NOT NULL,
    payload TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending', -- pending | in_flight | needs_reconciliation | failed
    error TEXT NOT NULL DEFAULT '',
    created INTEGER NOT NULL
);

CREATE TABLE content_versions (
    account TEXT NOT NULL,
    article INTEGER NOT NULL,
    version INTEGER NOT NULL,
    hash TEXT NOT NULL,              -- hash of the derived document
    raw_hash TEXT NOT NULL,          -- hash of the source content it was derived from
    dir TEXT NOT NULL,               -- relative directory inside the cache
    bytes INTEGER NOT NULL DEFAULT 0,
    created INTEGER NOT NULL,
    PRIMARY KEY (account, article, version)
) WITHOUT ROWID;

CREATE TABLE assets (
    account TEXT NOT NULL,
    article INTEGER NOT NULL,
    name TEXT NOT NULL,              -- hash-derived file name
    url TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending', -- pending | done | failed
    bytes INTEGER NOT NULL DEFAULT 0,
    tries INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account, article, name)
) WITHOUT ROWID;

CREATE TABLE leases (
    account TEXT NOT NULL,
    article INTEGER NOT NULL,
    version INTEGER NOT NULL,
    PRIMARY KEY (account, article, version)
) WITHOUT ROWID;

CREATE VIRTUAL TABLE article_fts USING fts5(
    account UNINDEXED, article UNINDEXED, title, body,
    tokenize = 'unicode61 remove_diacritics 2'
);
