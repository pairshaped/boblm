-- Add-only: no historical event scan or opportunity backfill.
ALTER TABLE idle_nudges ADD COLUMN recheck_at INTEGER;
CREATE INDEX idle_nudges_recheck ON idle_nudges(state, recheck_at, created_at);
