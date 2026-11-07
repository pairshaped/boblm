-- History pages skip transport-only deltas and lifecycle events.
CREATE INDEX events_history ON events(session, seq) WHERE
 kind IN ('message_appended', 'agent_session_changed') OR
 (kind = 'agent_event' AND (
  json_extract(payload, '$.type') IN ('assistant', 'user') OR
  (json_extract(payload, '$.type') = 'system' AND json_extract(payload, '$.subtype') = 'compact_boundary') OR
  (json_extract(payload, '$.type') = 'stream_event' AND json_extract(payload, '$.event.type') = 'message_start')
 ));
CREATE INDEX events_assistant_id ON events(session, json_extract(payload, '$.message.id'), seq)
 WHERE kind = 'agent_event' AND json_extract(payload, '$.type') = 'assistant';
CREATE INDEX events_stream_start ON events(session, seq)
 WHERE kind = 'agent_event' AND json_extract(payload, '$.type') = 'stream_event'
 AND json_extract(payload, '$.event.type') = 'message_start';
CREATE INDEX events_stream_id ON events(session, json_extract(payload, '$.event.message.id'), seq)
 WHERE kind = 'agent_event' AND json_extract(payload, '$.type') = 'stream_event'
 AND json_extract(payload, '$.event.type') = 'message_start';
CREATE INDEX events_secret_state ON events(session, kind, seq)
 WHERE kind IN ('secret_requested', 'secret_resolved');
CREATE INDEX events_secret_request_id ON events(session, json_extract(payload, '$.request_id'), seq)
 WHERE kind = 'secret_requested';
-- A result can be on a different history page from the call that named its tool.
CREATE TABLE tool_call_names (
 session TEXT NOT NULL REFERENCES sessions(id),
 id TEXT NOT NULL,
 name TEXT NOT NULL,
 PRIMARY KEY(session, id)
) STRICT;
INSERT OR IGNORE INTO tool_call_names(session, id, name)
 SELECT e.session, json_extract(CASE WHEN b.type = 'object' THEN b.value ELSE '{}' END, '$.id'), json_extract(CASE WHEN b.type = 'object' THEN b.value ELSE '{}' END, '$.name')
 FROM events e, json_each(CASE WHEN json_type(e.payload, '$.message.content') = 'array' THEN json_extract(e.payload, '$.message.content') ELSE '[]' END) b
 WHERE e.kind = 'agent_event' AND json_extract(e.payload, '$.type') = 'assistant'
 AND json_extract(CASE WHEN b.type = 'object' THEN b.value ELSE '{}' END, '$.type') = 'tool_use'
 AND json_type(CASE WHEN b.type = 'object' THEN b.value ELSE '{}' END, '$.id') = 'text' AND json_type(CASE WHEN b.type = 'object' THEN b.value ELSE '{}' END, '$.name') = 'text';
CREATE TRIGGER remember_tool_names AFTER INSERT ON events
 WHEN NEW.kind = 'agent_event' AND json_extract(NEW.payload, '$.type') = 'assistant'
 BEGIN
 INSERT OR REPLACE INTO tool_call_names(session, id, name)
 SELECT NEW.session, json_extract(CASE WHEN type = 'object' THEN value ELSE '{}' END, '$.id'), json_extract(CASE WHEN type = 'object' THEN value ELSE '{}' END, '$.name')
 FROM json_each(CASE WHEN json_type(NEW.payload, '$.message.content') = 'array' THEN json_extract(NEW.payload, '$.message.content') ELSE '[]' END)
 WHERE json_extract(CASE WHEN type = 'object' THEN value ELSE '{}' END, '$.type') = 'tool_use'
 AND json_type(CASE WHEN type = 'object' THEN value ELSE '{}' END, '$.id') = 'text' AND json_type(CASE WHEN type = 'object' THEN value ELSE '{}' END, '$.name') = 'text';
 END;
