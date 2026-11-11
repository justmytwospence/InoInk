-- Word count of the best text we hold, for reading-time estimates.
ALTER TABLE articles ADD COLUMN words INTEGER NOT NULL DEFAULT 0;
