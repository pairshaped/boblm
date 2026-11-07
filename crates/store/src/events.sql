-- func: append
INSERT INTO events (session, seq, kind, payload, created_at)
VALUES (@session, (SELECT COALESCE(MAX(seq), 0) + 1 FROM events WHERE session = @session), @kind, @payload, @created_at);

-- func: since
SELECT session, seq, kind, payload, created_at FROM events WHERE session = @session AND seq > @after ORDER BY seq LIMIT @limit;

-- func: recent_messages
SELECT session, seq, kind, payload, created_at FROM events
WHERE session = @session AND (
    kind = 'message_appended' OR
    (kind = 'claude_event' AND json_extract(payload, '$.type') IN ('assistant', 'user'))
)
ORDER BY seq DESC LIMIT @limit;

-- func: latest_compaction
SELECT payload FROM events WHERE session = @session AND kind = 'agent_event'
 AND json_extract(payload, '$.compaction_id') IS NOT NULL
ORDER BY seq DESC LIMIT 1;

-- func: last_assistant_text_seq
SELECT seq FROM events
WHERE session = @session AND kind = 'agent_event'
 AND json_extract(payload, '$.type') = 'assistant'
 AND EXISTS (
  SELECT 1 FROM json_each(json_extract(payload, '$.message.content')) AS block
  WHERE json_extract(block.value, '$.type') = 'text'
    AND length(COALESCE(json_extract(block.value, '$.text'), '')) > 0
 )
ORDER BY seq DESC LIMIT 1;

-- func: last_assistant_text
SELECT payload FROM events
WHERE session = @session AND kind = 'agent_event'
 AND json_extract(payload, '$.type') = 'assistant'
 AND EXISTS (
  SELECT 1 FROM json_each(json_extract(payload, '$.message.content')) AS block
  WHERE json_extract(block.value, '$.type') = 'text'
    AND length(COALESCE(json_extract(block.value, '$.text'), '')) > 0
 )
ORDER BY seq DESC LIMIT 1;

-- func: runner_restart_activity
SELECT payload, created_at FROM events
WHERE session = @session AND kind = 'agent_event' AND (
 json_extract(payload, '$.type') IN ('assistant', 'user', 'stream_event', 'result', 'task_started')
 OR (json_extract(payload, '$.type') = 'system' AND (
  json_extract(payload, '$.subtype') IN ('init', 'turn_started', 'task_started', 'compact_started')
  OR json_extract(payload, '$.status') IN ('requesting', 'compacting')
 ))
 OR (json_extract(payload, '$.type') = 'command_lifecycle'
  AND json_extract(payload, '$.state') = 'started')
)
ORDER BY seq DESC LIMIT 1;
