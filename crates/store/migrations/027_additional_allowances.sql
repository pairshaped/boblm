ALTER TABLE session_usage ADD COLUMN five_hour TEXT;
ALTER TABLE session_usage ADD COLUMN five_hour_at INTEGER;
ALTER TABLE session_usage ADD COLUMN credits TEXT;
ALTER TABLE session_usage ADD COLUMN credits_at INTEGER;

CREATE TRIGGER remember_additional_allowances AFTER INSERT ON events
WHEN NEW.kind = 'agent_event' AND json_extract(NEW.payload, '$.type') = 'usage'
BEGIN
 INSERT INTO session_usage(session, five_hour, five_hour_at, credits, credits_at)
 VALUES(NEW.session,
  CASE WHEN json_type(NEW.payload, '$.five_hour') = 'object' THEN json_extract(NEW.payload, '$.five_hour') END,
  CASE WHEN json_type(NEW.payload, '$.five_hour') = 'object' THEN NEW.created_at END,
  CASE WHEN json_type(NEW.payload, '$.credits') = 'object' THEN json_extract(NEW.payload, '$.credits') END,
  CASE WHEN json_type(NEW.payload, '$.credits') = 'object' THEN NEW.created_at END)
 ON CONFLICT(session) DO UPDATE SET
  five_hour = COALESCE(excluded.five_hour, session_usage.five_hour),
  five_hour_at = COALESCE(excluded.five_hour_at, session_usage.five_hour_at),
  credits = COALESCE(excluded.credits, session_usage.credits),
  credits_at = COALESCE(excluded.credits_at, session_usage.credits_at);
END;
