-- TradeLens schema v7: weekly and monthly reviews.
CREATE TABLE IF NOT EXISTS reviews (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER NOT NULL DEFAULT 0, -- 0: all accounts
    period     TEXT NOT NULL,              -- week, month
    start      TEXT NOT NULL,              -- YYYY-MM-DD
    json       TEXT NOT NULL,              -- PeriodReview without id
    UNIQUE (account_id, period, start)
);
