-- Add-only for live runners: their existing events queue error reports without new runner code.
-- result_seq identifies the source event for either kind; the result trigger stays unchanged.
ALTER TABLE subagent_reports ADD COLUMN source_kind TEXT NOT NULL DEFAULT 'result';
ALTER TABLE subagent_reports ADD COLUMN error_message TEXT;

CREATE TRIGGER queue_subagent_error_report AFTER INSERT ON events
WHEN NEW.kind IN ('driver_error', 'model_error', 'delivery_timeout')
BEGIN
 INSERT OR IGNORE INTO subagent_reports(
  child_session, result_seq, parent_session, state, created_at, source_kind, error_message)
 SELECT NEW.session, NEW.seq, parent_session_id, 'pending', NEW.created_at, NEW.kind,
  COALESCE(NULLIF(json_extract(NEW.payload, '$.message'), ''),
   CASE NEW.kind
    WHEN 'delivery_timeout' THEN 'The agent did not confirm message consumption before the deadline. Unconfirmed messages are held; inspect the chat before resuming.'
    ELSE 'The agent could not continue. Inspect its chat for the cause before resuming.'
   END)
 FROM sessions
 WHERE id = NEW.session AND parent_session_id IS NOT NULL
  AND NOT EXISTS (
   SELECT 1 FROM subagent_reports previous
   WHERE previous.child_session = NEW.session AND previous.source_kind != 'result'
    AND previous.error_message = COALESCE(NULLIF(json_extract(NEW.payload, '$.message'), ''),
     CASE NEW.kind
      WHEN 'delivery_timeout' THEN 'The agent did not confirm message consumption before the deadline. Unconfirmed messages are held; inspect the chat before resuming.'
      ELSE 'The agent could not continue. Inspect its chat for the cause before resuming.'
     END)
    AND previous.result_seq > COALESCE((
     SELECT MAX(seq) FROM events
     WHERE session = NEW.session AND kind = 'agent_event' AND seq < NEW.seq
      AND (json_extract(payload, '$.type') = 'result'
       OR (json_extract(payload, '$.type') = 'system'
        AND json_extract(payload, '$.subtype') = 'turn_started')
       OR (json_extract(payload, '$.type') = 'command_lifecycle'
        AND json_extract(payload, '$.state') = 'started'))
    ), 0)
  );
END;
