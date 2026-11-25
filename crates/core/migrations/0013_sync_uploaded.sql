-- The images known to be on a drive, by the name they have there (attachments/<sha256>.<ext>),
-- one set per provider: a sync only looks at the images this does not list, so one with nothing
-- new reads no image and lists no folder of them.
CREATE TABLE IF NOT EXISTS sync_uploaded (
    provider TEXT NOT NULL,
    name     TEXT NOT NULL,
    PRIMARY KEY (provider, name)
) WITHOUT ROWID;
