-- Codex account choice per chat. Add-only for live old runners.
ALTER TABLE sessions ADD COLUMN account_key TEXT;
