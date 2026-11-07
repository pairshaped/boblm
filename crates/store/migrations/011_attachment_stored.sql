ALTER TABLE attachments ADD COLUMN stored INTEGER NOT NULL DEFAULT 0 CHECK (stored IN (0, 1));
UPDATE attachments SET stored = 1;
