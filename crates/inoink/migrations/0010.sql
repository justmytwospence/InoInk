-- Inoreader's per-article data for saved (starred) articles: when it was saved and the reading
-- progress Inoreader reports (undocumented `starred` field and `savedProgress` object).
CREATE TABLE IF NOT EXISTS saved_items (
    account TEXT NOT NULL,
    article INTEGER NOT NULL,
    saved_at INTEGER,                 -- seconds; NULL when unknown
    kind TEXT,                        -- article | video | podcast; NULL until progress is reported
    length_secs INTEGER,              -- NULL when not reported
    percent INTEGER NOT NULL DEFAULT 0,
    finished INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account, article)
) WITHOUT ROWID;
-- How far the article was read on this device (furthest point, 0-100). Local only, never synced.
CREATE TABLE IF NOT EXISTS local_progress (
    account TEXT NOT NULL,
    article INTEGER NOT NULL,
    percent INTEGER NOT NULL,
    updated INTEGER NOT NULL,
    PRIMARY KEY (account, article)
) WITHOUT ROWID;
INSERT OR IGNORE INTO saved_items(account, article, saved_at)
SELECT account, id, CAST(json_extract(raw_json, '$.starred') AS INTEGER) FROM articles
WHERE json_valid(raw_json) AND CAST(json_extract(raw_json, '$.starred') AS INTEGER) > 0;
