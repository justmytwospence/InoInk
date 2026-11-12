-- Inoreader annotations (read-only, replaced whenever the article is fetched with annotations=1)
CREATE TABLE remote_annotations (
    account TEXT NOT NULL, article INTEGER NOT NULL, id INTEGER NOT NULL,
    start INTEGER NOT NULL DEFAULT 0, "end" INTEGER NOT NULL DEFAULT 0,
    text TEXT NOT NULL DEFAULT '', note TEXT NOT NULL DEFAULT '', added_on INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account, article, id)) WITHOUT ROWID;
-- Highlights made on the Kobo. Deleted ones stay as tombstones (deleted=1) for later sync.
CREATE TABLE highlights (
    account TEXT NOT NULL, uid TEXT NOT NULL, article INTEGER NOT NULL,
    exact TEXT NOT NULL, prefix TEXT NOT NULL DEFAULT '', suffix TEXT NOT NULL DEFAULT '',
    note TEXT NOT NULL DEFAULT '', content_hash TEXT NOT NULL DEFAULT '',
    start_loc INTEGER NOT NULL DEFAULT 0, end_loc INTEGER NOT NULL DEFAULT 0,
    created INTEGER NOT NULL, modified INTEGER NOT NULL, deleted INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account, uid)) WITHOUT ROWID;
CREATE INDEX highlights_article ON highlights (account, article, deleted);
-- Existing articles: copy annotations out of the stored API objects.
INSERT OR IGNORE INTO remote_annotations(account, article, id, start, "end", text, note, added_on)
SELECT a.account, a.id, CAST(json_extract(j.value,'$.id') AS INTEGER),
       COALESCE(json_extract(j.value,'$.start'),0), COALESCE(json_extract(j.value,'$.end'),0),
       COALESCE(json_extract(j.value,'$.text'),''), COALESCE(json_extract(j.value,'$.note'),''),
       COALESCE(json_extract(j.value,'$.added_on'),0)
FROM articles a,
     json_each(CASE WHEN json_valid(a.raw_json) THEN a.raw_json ELSE '{}' END, '$.annotations') j
WHERE json_type(CASE WHEN json_valid(a.raw_json) THEN a.raw_json ELSE '{}' END, '$.annotations') = 'array'
  AND json_extract(j.value,'$.id') IS NOT NULL;
