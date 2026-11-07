-- func: due
SELECT session, result_seq, created_at, state, message_id FROM idle_nudges
WHERE (state = 'pending' AND created_at <= @cutoff) OR state = 'queued'
ORDER BY created_at, session, result_seq;

-- func: lookup
SELECT state, message_id FROM idle_nudges WHERE session = @session AND result_seq = @result_seq;

-- func: settle
UPDATE idle_nudges SET state = @state, message_id = @message_id
WHERE session = @session AND result_seq = @result_seq;

-- func: inputs
WITH boundary AS (
 SELECT COALESCE(MAX(seq), 0) AS previous_result FROM events
 WHERE session = CAST(@session AS TEXT) AND seq < @result_seq AND kind = 'agent_event'
  AND json_extract(payload, '$.type') = 'result'
), tool_calls AS (
 SELECT e.seq, json_extract(block.value, '$.id') AS id
 FROM events e, json_each(e.payload, '$.message.content') block
 WHERE e.session = CAST(@session AS TEXT) AND e.seq < @result_seq AND e.kind = 'agent_event'
  AND json_extract(e.payload, '$.type') = 'assistant'
  AND CASE WHEN block.type = 'object' THEN json_extract(block.value, '$.type') = 'tool_use' ELSE 0 END
)
SELECT e.seq, e.payload,
 CAST(json_extract(a.payload, '$.sender_session_id') AS TEXT) AS sender,
 m.uuid, m.content, m.human_input
FROM events e
LEFT JOIN messages m ON m.session = e.session AND m.uuid = json_extract(e.payload, '$.command_uuid')
LEFT JOIN events a ON a.session = e.session AND a.kind = 'message_appended'
 AND json_extract(a.payload, '$.uuid') = m.uuid
WHERE e.session = CAST(@session AS TEXT) AND e.kind = 'agent_event' AND e.seq < @result_seq
 AND e.seq > (SELECT previous_result FROM boundary)
 AND ((json_extract(e.payload, '$.type') = 'command_lifecycle'
  AND json_extract(e.payload, '$.state') IN ('started', 'completed')
  AND NOT EXISTS (SELECT 1 FROM events previous
   WHERE previous.session = e.session AND previous.seq < e.seq AND previous.kind = 'agent_event'
    AND json_extract(previous.payload, '$.type') = 'command_lifecycle'
    AND json_extract(previous.payload, '$.command_uuid') = m.uuid
    AND json_extract(previous.payload, '$.state') IN ('started', 'completed')))
  OR (json_extract(e.payload, '$.type') = 'system'
   AND json_extract(e.payload, '$.subtype') = 'task_notification')
  OR json_extract(e.payload, '$.type') IN ('assistant', 'user', 'stream_event'))
 -- Codex can finish an earlier turn's background tool before acknowledging its job notice.
 -- Ignore only a nonempty batch consisting entirely of results proven to belong to older calls.
 AND NOT (json_extract(e.payload, '$.type') = 'user'
  AND EXISTS (SELECT 1 FROM sessions WHERE id = CAST(@session AS TEXT) AND agent = 'codex')
  AND COALESCE(json_type(e.payload, '$.message.content') = 'array', 0)
  AND COALESCE(json_array_length(e.payload, '$.message.content') > 0, 0)
  AND NOT EXISTS (SELECT 1 FROM json_each(e.payload, '$.message.content') block
   WHERE CASE WHEN block.type = 'object' THEN
    COALESCE(json_extract(block.value, '$.type'), '') != 'tool_result'
    OR COALESCE(json_type(block.value, '$.tool_use_id'), '') != 'text'
    OR COALESCE(json_extract(block.value, '$.tool_use_id'), '') = ''
    OR NOT EXISTS (SELECT 1 FROM tool_calls call
     WHERE call.seq < (SELECT previous_result FROM boundary) AND call.id = json_extract(block.value, '$.tool_use_id'))
    OR EXISTS (SELECT 1 FROM tool_calls call
     WHERE call.seq > (SELECT previous_result FROM boundary) AND call.seq < e.seq
      AND call.id = json_extract(block.value, '$.tool_use_id'))
   ELSE 1 END))
ORDER BY e.seq LIMIT 1;

-- func: report_sources
SELECT result_seq FROM subagent_reports WHERE child_session = @child AND parent_session = @parent;

-- func: eligible
SELECT EXISTS(SELECT 1 FROM sessions s
 WHERE s.id = @session AND s.archived = 0 AND s.parent_session_id IS NULL AND s.root_session_id IS NULL
 AND NOT EXISTS (SELECT 1 FROM runner_state WHERE session = @session AND stopped = 1)
 AND NOT EXISTS (SELECT 1 FROM messages WHERE session = @session AND expired_at IS NULL
  AND delivery_state IN ('queued', 'dispatching', 'uncertain', 'rejected'))
 AND NOT EXISTS (SELECT 1 FROM questions q JOIN sessions qs ON qs.id = q.session
  WHERE (qs.id = @session OR qs.parent_session_id = @session OR qs.root_session_id = @session)
   AND q.retired = 0 AND q.answer IS NULL AND q.discuss_later = 0)
 AND NOT EXISTS (SELECT 1 FROM subagent_reports WHERE parent_session = @session AND state = 'pending')
 AND NOT EXISTS (SELECT 1 FROM events e WHERE e.session = @session AND e.seq > @result_seq
  AND (e.kind IN ('message_appended', 'archive_changed', 'driver_error', 'model_error', 'delivery_timeout', 'session_stopping', 'session_stopped')
   OR (e.kind = 'agent_event' AND (
    json_extract(e.payload, '$.type') IN ('result', 'assistant', 'user', 'stream_event')
    OR (json_extract(e.payload, '$.type') = 'command_lifecycle' AND json_extract(e.payload, '$.state') = 'started')
    OR (json_extract(e.payload, '$.type') = 'system'
     AND json_extract(e.payload, '$.subtype') IN ('turn_started', 'task_notification', 'compact_started'))))))
) AS eligible;

-- func: guard_statuses
SELECT s.id, summary.payload,
 (SELECT COUNT(*) FROM events e WHERE e.session = s.id AND e.seq > MAX(COALESCE(summary.seq, 0), CASE WHEN s.id = @session THEN CAST(@result_seq AS INTEGER) ELSE 0 END)
   AND (e.kind = 'message_appended' OR (e.kind = 'agent_event'
    AND (json_extract(e.payload, '$.type') IN ('assistant', 'user', 'result', 'stream_event')
     OR (json_extract(e.payload, '$.type') = 'command_lifecycle' AND json_extract(e.payload, '$.state') = 'started')
     OR (json_extract(e.payload, '$.type') = 'background_jobs' AND (
      COALESCE(json_extract(e.payload, '$.snapshot.available'), 0) != 1
      OR (COALESCE(json_array_length(e.payload, '$.snapshot.tasks'), -1) != 0
       AND COALESCE(json_extract(e.payload, '$.snapshot.tasks') != json_extract(summary.payload, '$.jobs.tasks'), 1))))
     OR (json_extract(e.payload, '$.type') = 'system' AND (
      json_extract(e.payload, '$.subtype') IN ('turn_started', 'task_notification', 'compact_started')
      OR json_extract(e.payload, '$.status') = 'compacting')))))) AS unpublished
FROM sessions s LEFT JOIN events summary ON summary.session = s.id AND summary.seq = (
 SELECT MAX(seq) FROM events WHERE session = s.id AND kind = 'state_summary')
WHERE (s.id = @session OR s.parent_session_id = @session OR s.root_session_id = @session) AND s.archived = 0;

-- func: native_subagent_events
SELECT e.payload FROM subagent_events a JOIN events e ON e.session = a.session AND e.seq = a.seq
WHERE a.session = @session ORDER BY a.seq;

-- func: job_first_seen
SELECT first_seen FROM idle_nudge_jobs WHERE session = @session AND job_id = @job_id;
