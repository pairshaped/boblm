ALTER TABLE questions ADD COLUMN discuss_later INTEGER NOT NULL DEFAULT 0 CHECK (discuss_later IN (0, 1));
ALTER TABLE questions ADD COLUMN deferred_at INTEGER;
ALTER TABLE questions ADD COLUMN position INTEGER NOT NULL DEFAULT 0;
UPDATE questions SET position = rowid;
CREATE INDEX question_queue ON questions (session, discuss_later, answer, position);
