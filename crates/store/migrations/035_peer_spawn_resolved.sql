-- Role spawns store the resolved model, effort, account, step and reason on the
-- idempotency receipt so a retry never re-resolves. Add-only for old runners.
ALTER TABLE peer_spawn_requests ADD COLUMN resolved TEXT;
