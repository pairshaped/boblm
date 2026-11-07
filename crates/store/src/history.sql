-- func: cursor
SELECT CAST(COALESCE(MAX(seq), 0) AS INTEGER) AS cursor FROM events WHERE session = @session;

-- func: before
SELECT e.session, e.seq, e.kind, e.payload, e.created_at FROM events e
WHERE e.session = @session AND e.seq < @position AND e.seq <= @snapshot AND (
 e.kind IN ('message_appended', 'agent_session_changed') OR
 (e.kind = 'agent_event' AND (
  json_extract(e.payload, '$.type') IN ('assistant', 'user') OR
  (json_extract(e.payload, '$.type') = 'system' AND json_extract(e.payload, '$.subtype') IN ('compact_started', 'compact_boundary', 'compact_interrupted')) OR
  (json_extract(e.payload, '$.type') = 'stream_event' AND json_extract(e.payload, '$.event.type') = 'message_start')
 )))
 AND NOT (e.kind = 'agent_event' AND json_extract(e.payload, '$.type') = 'assistant'
  AND EXISTS (SELECT 1 FROM events a WHERE a.session = e.session AND a.kind = 'agent_event'
   AND json_extract(a.payload, '$.type') = 'stream_event' AND json_extract(a.payload, '$.event.type') = 'message_start'
   AND json_extract(a.payload, '$.event.message.id') = json_extract(e.payload, '$.message.id') AND a.seq <= e.seq))
 AND NOT (e.kind = 'agent_event' AND json_extract(e.payload, '$.type') = 'assistant'
  AND EXISTS (SELECT 1 FROM events a WHERE a.session = e.session AND a.kind = 'agent_event'
   AND json_extract(a.payload, '$.type') = 'assistant'
   AND json_extract(a.payload, '$.message.id') = json_extract(e.payload, '$.message.id')
   AND a.seq > e.seq AND a.seq <= @snapshot))
 AND NOT (e.kind = 'agent_event' AND json_extract(e.payload, '$.compaction_id') IS NOT NULL
  AND EXISTS (SELECT 1 FROM events a WHERE a.session = e.session AND a.kind = 'agent_event'
   AND json_extract(a.payload, '$.compaction_id') = json_extract(e.payload, '$.compaction_id') AND a.seq < e.seq))
ORDER BY e.seq DESC LIMIT @limit;

-- func: after
SELECT e.session, e.seq, e.kind, e.payload, e.created_at FROM events e
WHERE e.session = @session AND e.seq > @position AND e.seq <= @snapshot AND (
 e.kind IN ('message_appended', 'agent_session_changed') OR
 (e.kind = 'agent_event' AND (
  json_extract(e.payload, '$.type') IN ('assistant', 'user') OR
  (json_extract(e.payload, '$.type') = 'system' AND json_extract(e.payload, '$.subtype') IN ('compact_started', 'compact_boundary', 'compact_interrupted')) OR
  (json_extract(e.payload, '$.type') = 'stream_event' AND json_extract(e.payload, '$.event.type') = 'message_start')
 )))
 AND NOT (e.kind = 'agent_event' AND json_extract(e.payload, '$.type') = 'assistant'
  AND EXISTS (SELECT 1 FROM events a WHERE a.session = e.session AND a.kind = 'agent_event'
   AND json_extract(a.payload, '$.type') = 'stream_event' AND json_extract(a.payload, '$.event.type') = 'message_start'
   AND json_extract(a.payload, '$.event.message.id') = json_extract(e.payload, '$.message.id') AND a.seq <= e.seq))
 AND NOT (e.kind = 'agent_event' AND json_extract(e.payload, '$.type') = 'assistant'
  AND EXISTS (SELECT 1 FROM events a WHERE a.session = e.session AND a.kind = 'agent_event'
   AND json_extract(a.payload, '$.type') = 'assistant'
   AND json_extract(a.payload, '$.message.id') = json_extract(e.payload, '$.message.id')
   AND a.seq > e.seq AND a.seq <= @snapshot))
 AND NOT (e.kind = 'agent_event' AND json_extract(e.payload, '$.compaction_id') IS NOT NULL
  AND EXISTS (SELECT 1 FROM events a WHERE a.session = e.session AND a.kind = 'agent_event'
   AND json_extract(a.payload, '$.compaction_id') = json_extract(e.payload, '$.compaction_id') AND a.seq < e.seq))
ORDER BY e.seq ASC LIMIT @limit;

-- func: stream_text
SELECT CAST(COALESCE(group_concat(json_extract(payload, '$.event.delta.text'), ''), '') AS TEXT) AS text
FROM (SELECT payload FROM events WHERE session = @session AND seq > @start AND seq <= @snapshot
 AND seq < COALESCE((SELECT MIN(seq) FROM events WHERE session = @session AND seq > @start
  AND kind = 'agent_event' AND json_extract(payload, '$.type') = 'stream_event'
  AND json_extract(payload, '$.event.type') = 'message_start'), @snapshot + 1)
 AND kind = 'agent_event' AND json_extract(payload, '$.type') = 'stream_event'
 AND json_extract(payload, '$.event.type') = 'content_block_delta' ORDER BY seq);

-- func: tool_name
SELECT name FROM tool_call_names WHERE session = @session AND id = @id;

-- func: delivery
SELECT CASE WHEN expired_at IS NOT NULL THEN 'expired' ELSE delivery_state END AS delivery_state, expired_from FROM messages WHERE uuid = @uuid AND session = @session;

-- func: pending_secrets
SELECT e.session, e.seq, e.kind, e.payload, e.created_at FROM secret_requests r
JOIN events e ON e.session = r.session AND e.kind = 'secret_requested'
 AND json_extract(e.payload, '$.request_id') = r.id
WHERE r.session = @session AND r.state IN ('pending', 'saving') AND e.seq <= @snapshot
 AND NOT (e.kind = 'agent_event' AND json_extract(e.payload, '$.compaction_id') IS NOT NULL
  AND EXISTS (SELECT 1 FROM events a WHERE a.session = e.session AND a.kind = 'agent_event'
   AND json_extract(a.payload, '$.compaction_id') = json_extract(e.payload, '$.compaction_id') AND a.seq < e.seq))
ORDER BY e.seq;

-- func: completed
SELECT payload FROM events WHERE session = @session AND seq <= @snapshot
 AND kind = 'agent_event' AND json_extract(payload, '$.type') = 'assistant'
 AND json_extract(payload, '$.message.id') = @id ORDER BY seq DESC LIMIT 1;

-- func: current_stream
SELECT payload FROM events WHERE session = @session AND seq <= @snapshot
 AND kind = 'agent_event' AND json_extract(payload, '$.type') = 'stream_event'
 AND json_extract(payload, '$.event.type') = 'message_start' ORDER BY seq DESC LIMIT 1;

-- func: compaction_state
SELECT payload FROM events WHERE session = @session AND seq <= @snapshot AND kind = 'agent_event'
 AND json_extract(payload, '$.compaction_id') = @id ORDER BY seq DESC LIMIT 1;

-- func: subagent_events
SELECT e.session, e.seq, e.kind, e.payload, e.created_at
FROM subagent_events a JOIN events e ON e.session = a.session AND e.seq = a.seq
WHERE a.session = @session AND a.seq <= @snapshot ORDER BY a.seq;
