ALTER TABLE messages ADD COLUMN human_input INTEGER NOT NULL DEFAULT 0 CONSTRAINT boolean CHECK (human_input IN (0, 1));
