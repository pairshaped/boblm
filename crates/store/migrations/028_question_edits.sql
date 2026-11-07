ALTER TABLE questions ADD COLUMN retired INTEGER NOT NULL DEFAULT 0 CONSTRAINT boolean CHECK (retired IN (0, 1));
ALTER TABLE questions ADD COLUMN replacement_of TEXT;
