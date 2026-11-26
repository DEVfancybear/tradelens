-- High-impact economic news from Forex Factory's weekly feed. Public data each device fetches for
-- itself, so neither table is synced. A fetch replaces the events of its week; earlier weeks stay.
CREATE TABLE IF NOT EXISTS news_events (
    at_ms    INTEGER NOT NULL,
    currency TEXT    NOT NULL,
    title    TEXT    NOT NULL,
    forecast TEXT,
    previous TEXT,
    PRIMARY KEY (at_ms, currency, title)
) WITHOUT ROWID;

-- How the last fetches went: one row.
CREATE TABLE IF NOT EXISTS news_fetch (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    fetched_ms  INTEGER,
    error       TEXT,
    error_ms    INTEGER
);
