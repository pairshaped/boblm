-- Durable reports from a subagent's result events to its parent. Add-only for live old
-- runners: the trigger also queues results inserted by runners that have not restarted.
CREATE TABLE subagent_reports (
 child_session TEXT NOT NULL,
 result_seq INTEGER NOT NULL,
 parent_session TEXT NOT NULL,
 state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'delivered', 'failed')),
 error TEXT,
 created_at INTEGER NOT NULL,
 delivered_at INTEGER,
 PRIMARY KEY (child_session, result_seq)
) STRICT;

CREATE TRIGGER queue_subagent_report AFTER INSERT ON events
WHEN NEW.kind = 'agent_event'
 AND json_extract(NEW.payload, '$.type') = 'result'
BEGIN
 INSERT OR IGNORE INTO subagent_reports(
  child_session, result_seq, parent_session, state, created_at)
 SELECT NEW.session, NEW.seq, parent_session_id, 'pending', NEW.created_at
 FROM sessions
 WHERE id = NEW.session AND parent_session_id IS NOT NULL;
END;
