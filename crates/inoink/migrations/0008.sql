-- Per-feed full-text choice and what the automatic mode has learned.
CREATE TABLE IF NOT EXISTS feed_fulltext (
    account TEXT NOT NULL,
    feed TEXT NOT NULL,                 -- feed stream id (articles.origin_stream)
    mode TEXT NOT NULL DEFAULT 'auto',  -- auto | always | never
    tries INTEGER NOT NULL DEFAULT 0,
    gains INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account, feed)
) WITHOUT ROWID;
