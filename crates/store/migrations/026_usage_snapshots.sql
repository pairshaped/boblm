-- Keep the latest meter samples independent of the rendered history window.
CREATE TABLE session_usage (
 session TEXT PRIMARY KEY NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 context TEXT,
 context_at INTEGER,
 weekly TEXT,
 weekly_at INTEGER
) STRICT;

CREATE TRIGGER remember_usage AFTER INSERT ON events
WHEN NEW.kind = 'agent_event' AND json_extract(NEW.payload, '$.type') = 'usage'
BEGIN
 INSERT INTO session_usage(session, context, context_at, weekly, weekly_at)
 VALUES(NEW.session,
  CASE WHEN json_type(NEW.payload, '$.context') = 'object' THEN json_extract(NEW.payload, '$.context') END,
  CASE WHEN json_type(NEW.payload, '$.context') = 'object' THEN NEW.created_at END,
  CASE WHEN json_type(NEW.payload, '$.weekly') = 'object' THEN json_extract(NEW.payload, '$.weekly') END,
  CASE WHEN json_type(NEW.payload, '$.weekly') = 'object' THEN NEW.created_at END)
 ON CONFLICT(session) DO UPDATE SET
  context = COALESCE(excluded.context, session_usage.context),
  context_at = COALESCE(excluded.context_at, session_usage.context_at),
  weekly = COALESCE(excluded.weekly, session_usage.weekly),
  weekly_at = COALESCE(excluded.weekly_at, session_usage.weekly_at);
END;

CREATE TRIGGER clear_conversation_usage AFTER INSERT ON events
WHEN (NEW.kind = 'agent_session_changed' AND json_type(NEW.payload, '$.previous') = 'text')
 OR (NEW.kind = 'agent_event' AND json_extract(NEW.payload, '$.type') = 'system'
  AND json_extract(NEW.payload, '$.subtype') IN ('compact_started', 'compact_boundary'))
BEGIN
 UPDATE session_usage SET context = NULL, context_at = NULL WHERE session = NEW.session;
END;

CREATE TRIGGER clear_model_usage AFTER UPDATE OF model ON sessions
WHEN OLD.model IS NOT NEW.model
BEGIN
 UPDATE session_usage SET context = NULL, context_at = NULL WHERE session = NEW.id;
END;
