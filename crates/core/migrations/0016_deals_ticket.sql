-- Deals looked up by their terminal ticket alone, across accounts: the account holding the same
-- deals as one being deleted (`views::twin_account`).
CREATE INDEX IF NOT EXISTS deals_ticket ON deals (ticket);
