-- TradeLens schema v4: the trader's own option lists and fields.

CREATE TABLE IF NOT EXISTS catalog_items (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    kind     TEXT NOT NULL,              -- tag, mistake, emotion, exit_reason, setup_grade, market_condition, timeframe
    key      TEXT NOT NULL,
    label    TEXT NOT NULL,
    color    TEXT,
    position INTEGER NOT NULL DEFAULT 0,
    archived INTEGER NOT NULL DEFAULT 0,
    UNIQUE (kind, key)
);

CREATE TABLE IF NOT EXISTS field_defs (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    scope      TEXT NOT NULL,            -- trade, entry_plan, day, review, missed_trade
    key        TEXT NOT NULL,
    def_json   TEXT NOT NULL,            -- FieldDef without id
    position   INTEGER NOT NULL DEFAULT 0,
    archived   INTEGER NOT NULL DEFAULT 0,
    UNIQUE (scope, key)
);
