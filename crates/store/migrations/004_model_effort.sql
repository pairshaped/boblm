ALTER TABLE sessions ADD COLUMN model TEXT NOT NULL DEFAULT 'sonnet' CHECK (length(model) > 0);
ALTER TABLE sessions ADD COLUMN effort TEXT CHECK (effort IS NULL OR length(effort) > 0);
