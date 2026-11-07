-- Every runner now reads allowances from account_usage, so the per-chat allowance columns
-- and the triggers that kept them for old runners (ADR 0037, ADR 0040) are no longer needed.
DROP TRIGGER remember_additional_allowances;
DROP TRIGGER remember_usage;

ALTER TABLE session_usage DROP COLUMN weekly;
ALTER TABLE session_usage DROP COLUMN weekly_at;
ALTER TABLE session_usage DROP COLUMN five_hour;
ALTER TABLE session_usage DROP COLUMN five_hour_at;
ALTER TABLE session_usage DROP COLUMN credits;
ALTER TABLE session_usage DROP COLUMN credits_at;

CREATE TRIGGER remember_usage AFTER INSERT ON events
WHEN NEW.kind = 'agent_event' AND json_extract(NEW.payload, '$.type') = 'usage'
 AND json_type(NEW.payload, '$.context') = 'object'
BEGIN
 INSERT INTO session_usage(session, context, context_at)
 VALUES(NEW.session, json_extract(NEW.payload, '$.context'), NEW.created_at)
 ON CONFLICT(session) DO UPDATE SET
  context = excluded.context,
  context_at = excluded.context_at;
END;
