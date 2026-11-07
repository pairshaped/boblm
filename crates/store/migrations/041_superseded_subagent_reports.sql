-- A pending report whose child finished again before it was delivered is superseded by the
-- newer one. Add-only for live old runners: the table keeps its name and columns, and the
-- queue trigger is recreated unchanged in the same transaction, so a runner that has not
-- restarted still queues 'pending' rows. Old web code only reads the state as text.
DROP TRIGGER queue_subagent_report;
CREATE TABLE subagent_reports_new (
 child_session TEXT NOT NULL,
 result_seq INTEGER NOT NULL,
 parent_session TEXT NOT NULL,
 state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'delivered', 'failed', 'superseded')),
 error TEXT,
 created_at INTEGER NOT NULL,
 delivered_at INTEGER,
 PRIMARY KEY (child_session, result_seq)
) STRICT;
INSERT INTO subagent_reports_new SELECT * FROM subagent_reports;
DROP TABLE subagent_reports;
ALTER TABLE subagent_reports_new RENAME TO subagent_reports;

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
