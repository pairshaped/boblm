-- Allowance readings belong to the provider account, not one chat. This migration is
-- additive on purpose: live old runners still read the per-chat allowance columns and
-- their triggers until each runner has restarted on new code (ADR 0040).
CREATE TABLE account_usage (
 provider TEXT NOT NULL,
 account_id TEXT NOT NULL,
 five_hour_used REAL,
 five_hour_resets_at INTEGER,
 five_hour_observed_at INTEGER,
 weekly_used REAL,
 weekly_resets_at INTEGER,
 weekly_observed_at INTEGER,
 credits_balance REAL,
 credits_observed_at INTEGER,
 PRIMARY KEY (provider, account_id)
) STRICT;

ALTER TABLE sessions ADD COLUMN account_id TEXT;
