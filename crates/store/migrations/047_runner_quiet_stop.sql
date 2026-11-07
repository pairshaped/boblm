ALTER TABLE runner_state ADD COLUMN quiet_stop INTEGER NOT NULL DEFAULT 0
 CONSTRAINT boolean CHECK (quiet_stop IN (0, 1));
