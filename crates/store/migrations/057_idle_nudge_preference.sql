ALTER TABLE sessions ADD COLUMN idle_nudge_enabled INTEGER NOT NULL DEFAULT 1
 CONSTRAINT boolean CHECK (idle_nudge_enabled IN (0, 1));

ALTER TABLE messages ADD COLUMN expiry_reason TEXT;

-- Widen the list event vocabulary while keeping existing runner writes compatible.
CREATE TABLE session_list_events_new (
 seq INTEGER PRIMARY KEY AUTOINCREMENT,
 session TEXT NOT NULL,
 kind TEXT NOT NULL CHECK (kind IN ('created', 'renamed', 'archived', 'state_summary', 'deleted', 'subagents_changed', 'questions_changed', 'idle_nudge_changed')),
 payload TEXT NOT NULL CHECK (json_valid(payload)),
 created_at INTEGER NOT NULL
) STRICT;
INSERT INTO session_list_events_new SELECT * FROM session_list_events;
DELETE FROM sqlite_sequence WHERE name = 'session_list_events_new';
INSERT INTO sqlite_sequence (name, seq)
SELECT 'session_list_events_new', seq FROM sqlite_sequence WHERE name = 'session_list_events';
DROP TABLE session_list_events;
ALTER TABLE session_list_events_new RENAME TO session_list_events;

DROP TRIGGER queue_idle_nudge;
CREATE TRIGGER queue_idle_nudge AFTER INSERT ON events
WHEN NEW.kind = 'agent_event' AND json_extract(NEW.payload, '$.type') = 'result'
BEGIN
 INSERT INTO idle_nudges(session, result_seq, created_at)
 SELECT NEW.session, NEW.seq, NEW.created_at FROM sessions
 WHERE id = NEW.session AND parent_session_id IS NULL AND root_session_id IS NULL
  AND idle_nudge_enabled = 1;
END;
