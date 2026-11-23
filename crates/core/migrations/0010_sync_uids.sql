-- TradeLens schema v10: identities that mean the same on every device, and the sync's bookkeeping.
--
-- Row ids differ per device, so what has no natural key gets a random one (a uid): playbooks,
-- missed trades and checklist items. Everything else is keyed by what the trader sees: an
-- account by login and server, a trade by account, position and segment, a day note by day...
-- (reviews already have such a key: account, period and start).

-- Two devices may each make a setup of the same name, and sync must keep both: the name is no
-- longer unique in the table (saving a setup by hand still refuses a name that is taken).
-- The table is rebuilt for that; its ids, and the numbers its ids never go back to, are kept.
CREATE TABLE playbooks_next (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    uid      TEXT,
    name     TEXT NOT NULL,
    json     TEXT NOT NULL,
    position INTEGER NOT NULL DEFAULT 0,
    archived INTEGER NOT NULL DEFAULT 0
);
INSERT INTO playbooks_next (id, uid, name, json, position, archived)
    SELECT id, lower(hex(randomblob(16))), name, json, position, archived FROM playbooks;
DELETE FROM sqlite_sequence WHERE name = 'playbooks_next';
INSERT INTO sqlite_sequence (name, seq) SELECT 'playbooks_next', seq FROM sqlite_sequence WHERE name = 'playbooks';
DROP TABLE playbooks;
ALTER TABLE playbooks_next RENAME TO playbooks;

ALTER TABLE missed_trades ADD COLUMN uid TEXT;
ALTER TABLE checklist ADD COLUMN uid TEXT;

UPDATE missed_trades SET uid = lower(hex(randomblob(16))) WHERE uid IS NULL;
UPDATE checklist SET uid = lower(hex(randomblob(16))) WHERE uid IS NULL;

CREATE UNIQUE INDEX playbooks_uid ON playbooks (uid);
CREATE UNIQUE INDEX missed_trades_uid ON missed_trades (uid);
CREATE UNIQUE INDEX checklist_uid ON checklist (uid);

-- Whatever inserts a row gets a uid, so no code path can forget one.
CREATE TRIGGER playbooks_new_uid AFTER INSERT ON playbooks WHEN NEW.uid IS NULL BEGIN
    UPDATE playbooks SET uid = lower(hex(randomblob(16))) WHERE id = NEW.id;
END;
CREATE TRIGGER missed_trades_new_uid AFTER INSERT ON missed_trades WHEN NEW.uid IS NULL BEGIN
    UPDATE missed_trades SET uid = lower(hex(randomblob(16))) WHERE id = NEW.id;
END;
CREATE TRIGGER checklist_new_uid AFTER INSERT ON checklist WHEN NEW.uid IS NULL BEGIN
    UPDATE checklist SET uid = lower(hex(randomblob(16))) WHERE id = NEW.id;
END;

-- The first sync log wrote journal rows under local ids and was never pushed: start it again.
DELETE FROM changes;

-- When each field was last written, and by which device: what decides a merge.
CREATE TABLE sync_clock (
    entity    TEXT NOT NULL,
    key       TEXT NOT NULL,
    field     TEXT NOT NULL,
    at        INTEGER NOT NULL,
    device_id TEXT NOT NULL,
    PRIMARY KEY (entity, key, field)
) WITHOUT ROWID;

-- Entities deleted, with their content when they were: a later edit from another device brings
-- them back as they were.
CREATE TABLE sync_tombstones (
    entity    TEXT NOT NULL,
    key       TEXT NOT NULL,
    at        INTEGER NOT NULL,
    device_id TEXT NOT NULL,
    data      TEXT NOT NULL,   -- the entity's fields, JSON
    PRIMARY KEY (entity, key)
) WITHOUT ROWID;

-- Images by content hash: what the file is called in the cloud (attachments/<sha256>.<ext>).
CREATE TABLE sync_attachments (
    path   TEXT PRIMARY KEY,   -- relative to the data folder, as the journal refers to it
    sha256 TEXT NOT NULL
);
