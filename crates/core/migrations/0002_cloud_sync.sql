-- TradeLens schema v2: the change log that cloud sync pushes and pulls.

CREATE TABLE IF NOT EXISTS changes (
    device_id TEXT NOT NULL,
    seq       INTEGER NOT NULL,
    at        INTEGER NOT NULL,
    entity    TEXT NOT NULL,
    key       TEXT NOT NULL,
    value     TEXT,             -- JSON; NULL deletes
    PRIMARY KEY (device_id, seq)
);
CREATE INDEX IF NOT EXISTS changes_entity ON changes (entity, key);

CREATE TABLE IF NOT EXISTS cloud_state (
    provider     TEXT PRIMARY KEY, -- Google, Microsoft
    email        TEXT NOT NULL DEFAULT '',
    name         TEXT NOT NULL DEFAULT '',
    pushed_seq   INTEGER NOT NULL DEFAULT 0,
    pulled_json  TEXT NOT NULL DEFAULT '{}', -- {device_id: last seq applied}
    last_sync    INTEGER,
    auto_sync    INTEGER NOT NULL DEFAULT 1
);
