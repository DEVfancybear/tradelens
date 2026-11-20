-- TradeLens schema v3: the reason of each deal (ENUM_DEAL_REASON), for exit reasons.
ALTER TABLE deals ADD COLUMN reason TEXT;
