ALTER TABLE sessions ADD COLUMN applied_model TEXT NOT NULL DEFAULT 'sonnet' CHECK (length(applied_model) > 0);
ALTER TABLE sessions ADD COLUMN applied_effort TEXT CHECK (applied_effort IS NULL OR length(applied_effort) > 0);
UPDATE sessions SET applied_model = model, applied_effort = effort;
