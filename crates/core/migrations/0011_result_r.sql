-- TradeLens schema v11: the result of a deal in R, for trades imported with only their R (no money).
ALTER TABLE deals ADD COLUMN result_r REAL;
