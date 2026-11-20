-- TradeLens schema v6: setups seen and not taken.
CREATE TABLE IF NOT EXISTS missed_trades (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER REFERENCES accounts(id) ON DELETE SET NULL,
    seen_at    INTEGER NOT NULL,
    json       TEXT NOT NULL             -- MissedTrade without id
);
CREATE INDEX IF NOT EXISTS missed_trades_seen ON missed_trades (seen_at);
