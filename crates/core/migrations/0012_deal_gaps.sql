-- TradeLens schema v12: what a deal's source did not tell (see `model::Gaps`): 1 the time of day,
-- 2 the lot size, 4 the prices. Existing journal imports are brought up to what is certain about them.
ALTER TABLE deals ADD COLUMN gaps INTEGER NOT NULL DEFAULT 0;
-- A journal import without a price column stored 0 as the price, and the default lot (1) where the file
-- had none. A real fill never has price 0.
UPDATE deals SET gaps = gaps | 4 WHERE source = 'notion' AND price = 0;
UPDATE deals SET gaps = gaps | 2 WHERE source = 'notion' AND price = 0 AND volume = 1.0;
