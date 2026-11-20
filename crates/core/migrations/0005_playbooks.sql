-- TradeLens schema v5: the playbook of setups.
CREATE TABLE IF NOT EXISTS playbooks (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    name     TEXT NOT NULL UNIQUE,
    json     TEXT NOT NULL,              -- Playbook without id
    position INTEGER NOT NULL DEFAULT 0,
    archived INTEGER NOT NULL DEFAULT 0
);
