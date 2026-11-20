-- TradeLens schema v1. Deals are the source of truth; trades are rebuilt from them.

CREATE TABLE IF NOT EXISTS accounts (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    login             TEXT NOT NULL,
    server            TEXT NOT NULL DEFAULT '',
    broker            TEXT NOT NULL DEFAULT '',
    name              TEXT NOT NULL DEFAULT '',
    currency          TEXT NOT NULL DEFAULT 'USD',
    initial_balance   REAL,
    last_balance      REAL,
    last_equity       REAL,
    server_offset_min INTEGER,
    last_sync         INTEGER,
    UNIQUE (login, server)
);

CREATE TABLE IF NOT EXISTS deals (
    account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    ticket      INTEGER NOT NULL,          -- negative for manual trades
    order_id    INTEGER NOT NULL DEFAULT 0,
    position_id INTEGER NOT NULL DEFAULT 0,
    time_msc    INTEGER NOT NULL,          -- broker server time, ms
    symbol      TEXT NOT NULL DEFAULT '',
    kind        TEXT NOT NULL,             -- buy, sell, balance, credit, ...
    entry       TEXT NOT NULL DEFAULT '',  -- in, out, inout, out_by
    volume      REAL NOT NULL DEFAULT 0,
    price       REAL NOT NULL DEFAULT 0,
    commission  REAL NOT NULL DEFAULT 0,
    swap        REAL NOT NULL DEFAULT 0,
    fee         REAL NOT NULL DEFAULT 0,
    profit      REAL NOT NULL DEFAULT 0,
    magic       INTEGER NOT NULL DEFAULT 0,
    comment     TEXT NOT NULL DEFAULT '',
    sl          REAL NOT NULL DEFAULT 0,
    tp          REAL NOT NULL DEFAULT 0,
    risk        REAL,
    source      TEXT NOT NULL DEFAULT 'ea', -- ea, python, html, xlsx, json, manual
    PRIMARY KEY (account_id, ticket)
);
CREATE INDEX IF NOT EXISTS deals_position ON deals (account_id, position_id);
CREATE INDEX IF NOT EXISTS deals_time ON deals (account_id, time_msc);

-- One journal entry per trade (position + segment). Lists are JSON arrays.
CREATE TABLE IF NOT EXISTS journal (
    account_id     INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    position_id    INTEGER NOT NULL,
    segment        INTEGER NOT NULL DEFAULT 0,
    entry_json     TEXT NOT NULL,
    updated_at     INTEGER NOT NULL,
    PRIMARY KEY (account_id, position_id, segment)
);

CREATE TABLE IF NOT EXISTS rules (
    account_id             INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    max_daily_loss         REAL,
    max_daily_trades       INTEGER,
    max_risk_pct           REAL,
    max_consecutive_losses INTEGER,
    daily_target           REAL
);

CREATE TABLE IF NOT EXISTS checklist (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER,                    -- NULL: every account
    text       TEXT NOT NULL,
    position   INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS day_notes (
    account_id INTEGER NOT NULL DEFAULT 0, -- 0: all accounts
    day        TEXT NOT NULL,              -- YYYY-MM-DD, local
    plan       TEXT NOT NULL DEFAULT '',
    review     TEXT NOT NULL DEFAULT '',
    mood       TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (account_id, day)
);

-- Sync token, port, display time zone, catalogs of setups / tags / mistakes.
CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS import_log (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    at         INTEGER NOT NULL,
    source     TEXT NOT NULL,
    account_id INTEGER,
    inserted   INTEGER NOT NULL DEFAULT 0,
    updated    INTEGER NOT NULL DEFAULT 0,
    message    TEXT NOT NULL DEFAULT ''
);
