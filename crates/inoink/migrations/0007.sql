-- 0007: titles, authors and feed names decoded from HTML entities (done in Rust by
-- Store::backfill_titles after this migration).
UPDATE meta SET value = value WHERE 0;
