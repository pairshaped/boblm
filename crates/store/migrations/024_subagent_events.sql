-- Keep lifecycle lookup independent of the browser's retained message window.
CREATE TABLE subagent_events (
 session TEXT NOT NULL,
 seq INTEGER NOT NULL,
 PRIMARY KEY(session, seq),
 FOREIGN KEY(session, seq) REFERENCES events(session, seq) ON DELETE CASCADE
) STRICT;

INSERT OR IGNORE INTO subagent_events(session, seq)
SELECT e.session, e.seq FROM events e
WHERE e.kind = 'agent_event' AND EXISTS (
 SELECT 1 FROM json_each(CASE WHEN json_type(e.payload, '$.message.content') = 'array' THEN json_extract(e.payload, '$.message.content') ELSE '[]' END) b WHERE (
 (json_extract(CASE WHEN b.type = 'object' THEN b.value ELSE '{}' END, '$.type') = 'tool_use'
  AND lower(json_extract(CASE WHEN b.type = 'object' THEN b.value ELSE '{}' END, '$.name')) IN ('agent', 'task', 'subagentactivity')) OR
 (json_extract(CASE WHEN b.type = 'object' THEN b.value ELSE '{}' END, '$.type') = 'tool_result' AND EXISTS (
   SELECT 1 FROM tool_call_names t WHERE t.session = e.session
    AND t.id = json_extract(CASE WHEN b.type = 'object' THEN b.value ELSE '{}' END, '$.tool_use_id')
    AND lower(t.name) IN ('agent', 'task', 'subagentactivity')))
)
);

CREATE TRIGGER remember_subagent_events AFTER INSERT ON events
WHEN NEW.kind = 'agent_event'
BEGIN
 INSERT OR IGNORE INTO subagent_events(session, seq)
 SELECT NEW.session, NEW.seq WHERE EXISTS (
  SELECT 1 FROM json_each(CASE WHEN json_type(NEW.payload, '$.message.content') = 'array' THEN json_extract(NEW.payload, '$.message.content') ELSE '[]' END) b
  WHERE (
 (json_extract(CASE WHEN b.type = 'object' THEN b.value ELSE '{}' END, '$.type') = 'tool_use'
  AND lower(json_extract(CASE WHEN b.type = 'object' THEN b.value ELSE '{}' END, '$.name')) IN ('agent', 'task', 'subagentactivity')) OR
 (json_extract(CASE WHEN b.type = 'object' THEN b.value ELSE '{}' END, '$.type') = 'tool_result' AND EXISTS (
   SELECT 1 FROM tool_call_names t WHERE t.session = NEW.session
    AND t.id = json_extract(CASE WHEN b.type = 'object' THEN b.value ELSE '{}' END, '$.tool_use_id')
    AND lower(t.name) IN ('agent', 'task', 'subagentactivity')))
)
 );
END;
