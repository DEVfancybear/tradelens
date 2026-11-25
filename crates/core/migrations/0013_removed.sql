-- A migration 0013 once existed (it inferred "date-only trade" from a 00:00 time) and was removed.
-- Databases that ran it are at user_version 13 and the next file must still be run on them, so the
-- number stays taken by this no-op.
SELECT 1;
