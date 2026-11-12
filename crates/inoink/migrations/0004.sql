-- Link posts from discussion sites: the extracted article and the cached comments page.
CREATE TABLE discussions (
    account TEXT NOT NULL,
    article INTEGER NOT NULL,
    site TEXT NOT NULL,                    -- 'hn' or 'lobsters'
    ext_id TEXT NOT NULL,                  -- item id / short id on the site
    url TEXT NOT NULL,                     -- human comments page
    article_url TEXT NOT NULL DEFAULT '',  -- linked article; '' for text posts
    article_html TEXT,                     -- extracted or post HTML once obtained
    comment_count INTEGER NOT NULL DEFAULT 0,
    fetched INTEGER NOT NULL DEFAULT 0,    -- unix time of the comments snapshot; 0 = none
    comments_bytes INTEGER NOT NULL DEFAULT 0,
    pos_hash TEXT NOT NULL DEFAULT '',
    pos_offset INTEGER NOT NULL DEFAULT 0,
    pos_anchor TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (account, article)
) WITHOUT ROWID;
