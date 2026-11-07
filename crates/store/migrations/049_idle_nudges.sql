-- Add-only: loaded runners queue candidates through their existing result writes.
CREATE TABLE idle_nudges (
 session TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 result_seq INTEGER NOT NULL,
 created_at INTEGER NOT NULL,
 state TEXT NOT NULL DEFAULT 'pending',
 message_id TEXT,
 PRIMARY KEY(session, result_seq)
) STRICT;

CREATE INDEX idle_nudges_due ON idle_nudges(state, created_at);

CREATE TRIGGER queue_idle_nudge AFTER INSERT ON events
WHEN NEW.kind = 'agent_event' AND json_extract(NEW.payload, '$.type') = 'result'
BEGIN
 INSERT INTO idle_nudges(session, result_seq, created_at)
 SELECT NEW.session, NEW.seq, NEW.created_at FROM sessions
 WHERE id = NEW.session AND parent_session_id IS NULL AND root_session_id IS NULL;
END;
