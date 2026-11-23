-- Inoreader's Archive: Undo of Archive and Remove from Archive restores the article's place.
-- (Rebuilt rather than altered: the undo history is short-lived and this can be re-run.)
DROP TABLE IF EXISTS undos;
CREATE TABLE undos (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account TEXT NOT NULL,
    article INTEGER NOT NULL,
    prev_read INTEGER NOT NULL,
    prev_starred INTEGER NOT NULL,
    prev_archived INTEGER NOT NULL DEFAULT 0,
    created INTEGER NOT NULL,
    used INTEGER NOT NULL DEFAULT 0
);
-- Liked and Broadcast left Inoreader's interface in 2025; InoInk no longer tracks them.
DELETE FROM intents WHERE key IN ('like','broadcast');
DELETE FROM remote_state WHERE key IN ('like','broadcast');
