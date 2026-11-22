-- The values of the trader's own day fields (scope `day`), as JSON by field key, kept on the
-- day's note so that they sync and are searched with it.
ALTER TABLE day_notes ADD COLUMN custom_json TEXT NOT NULL DEFAULT '{}';
