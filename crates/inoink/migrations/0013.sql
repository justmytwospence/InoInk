-- Per article: what fetching its full text from the original page gave. No row: not decided yet.
CREATE TABLE IF NOT EXISTS article_fulltext (
    account TEXT NOT NULL,
    article INTEGER NOT NULL,
    state TEXT NOT NULL,               -- done | skip | failed | retry
    tries INTEGER NOT NULL DEFAULT 0,  -- temporary failures counted so far
    PRIMARY KEY (account, article)
) WITHOUT ROWID;
-- Articles already showing their original page (extracted, or a link post's article).
INSERT OR IGNORE INTO article_fulltext(account, article, state)
SELECT DISTINCT account, article, 'done' FROM content_versions
WHERE raw_hash LIKE 'extract:%' OR raw_hash LIKE 'post:%';
