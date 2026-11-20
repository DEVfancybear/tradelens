-- TradeLens schema v8: full-text search over what the trader writes.
-- remove_diacritics folds Vietnamese accents: "lenh" matches "lệnh".
CREATE VIRTUAL TABLE IF NOT EXISTS search_index USING fts5(
    kind UNINDEXED,
    reference UNINDEXED,
    title,
    body,
    tokenize = 'unicode61 remove_diacritics 2'
);
