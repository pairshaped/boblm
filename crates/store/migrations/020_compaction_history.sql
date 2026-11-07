DROP INDEX events_history;
CREATE INDEX events_history ON events(session, seq) WHERE
 kind IN ('message_appended', 'agent_session_changed') OR
 (kind = 'agent_event' AND (
  json_extract(payload, '$.type') IN ('assistant', 'user') OR
  (json_extract(payload, '$.type') = 'system' AND json_extract(payload, '$.subtype') IN ('compact_started', 'compact_boundary', 'compact_interrupted')) OR
  (json_extract(payload, '$.type') = 'stream_event' AND json_extract(payload, '$.event.type') = 'message_start')
 ));
CREATE INDEX events_compaction_latest ON events(session, seq)
 WHERE kind = 'agent_event' AND json_extract(payload, '$.compaction_id') IS NOT NULL;
CREATE INDEX events_compaction_id ON events(session, json_extract(payload, '$.compaction_id'), seq)
 WHERE kind = 'agent_event' AND json_extract(payload, '$.compaction_id') IS NOT NULL;
