-- Links and articles waiting to be sent to another service (Karakeep).
CREATE TABLE shares (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account TEXT NOT NULL,
    target TEXT NOT NULL,
    url TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL DEFAULT 'pending',   -- pending | failed; sent rows are deleted
    error TEXT NOT NULL DEFAULT '',
    tries INTEGER NOT NULL DEFAULT 0,
    created INTEGER NOT NULL
);
CREATE INDEX shares_state ON shares (account, state, id);
