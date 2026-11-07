ALTER TABLE sessions ADD COLUMN name TEXT NOT NULL DEFAULT '' CHECK (length(name) <= 200);
