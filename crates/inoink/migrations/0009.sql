-- Headlines seen on this device (local only, never synced) and undo for bulk actions.
CREATE TABLE IF NOT EXISTS seen (
    account TEXT NOT NULL,
    article INTEGER NOT NULL,
    at INTEGER NOT NULL,                -- microseconds since the epoch: first shown in a list or opened
    PRIMARY KEY (account, article)
) WITHOUT ROWID;
-- A bulk selection that was applied: the membership key it changed ('read' or 'starred').
CREATE TABLE IF NOT EXISTS bulk_applied (
    scope INTEGER PRIMARY KEY REFERENCES bulk_scopes(id) ON DELETE CASCADE,
    membership TEXT NOT NULL,
    undone INTEGER NOT NULL DEFAULT 0
);
-- Each article's value before a bulk action, for undo.
CREATE TABLE IF NOT EXISTS bulk_prev (
    scope INTEGER NOT NULL REFERENCES bulk_scopes(id) ON DELETE CASCADE,
    article INTEGER NOT NULL,
    prev INTEGER NOT NULL,
    PRIMARY KEY (scope, article)
) WITHOUT ROWID;
