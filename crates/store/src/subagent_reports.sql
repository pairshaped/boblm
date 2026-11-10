-- func: all
SELECT child_session, result_seq, parent_session, state, error, created_at, delivered_at, source_kind, error_message
FROM subagent_reports ORDER BY created_at, child_session, result_seq;

-- func: pending
SELECT child_session, result_seq, parent_session, state, error, created_at, delivered_at, source_kind, error_message
FROM subagent_reports WHERE state = 'pending'
ORDER BY created_at, child_session, result_seq;

-- func: due
SELECT child_session, result_seq, parent_session, state, error, created_at, delivered_at, source_kind, error_message
FROM subagent_reports WHERE state = 'pending' AND recheck_at IS NULL
UNION ALL
SELECT child_session, result_seq, parent_session, state, error, created_at, delivered_at, source_kind, error_message
FROM subagent_reports WHERE state = 'pending' AND recheck_at <= @now
ORDER BY created_at, child_session, result_seq;

-- func: latest_inventory
SELECT CAST(CASE WHEN kind = 'state_summary' THEN json_extract(payload, '$.jobs')
 ELSE json_extract(payload, '$.snapshot') END AS TEXT) AS jobs
FROM events WHERE session = @child_session
 AND (kind = 'state_summary' OR (kind = 'agent_event' AND json_extract(payload, '$.type') = 'background_jobs'))
ORDER BY seq DESC LIMIT 1;

-- func: defer
UPDATE subagent_reports SET recheck_at = @recheck_at, recheck_jobs = @recheck_jobs
WHERE child_session = @child_session AND result_seq = @result_seq AND state = 'pending';

-- func: accepted_delivery
SELECT m.content, r.delivered_text_seq
FROM subagent_reports r JOIN messages m ON m.session = r.parent_session AND m.uuid = @message_id
WHERE r.child_session = @child_session AND r.result_seq = @result_seq
 AND EXISTS (SELECT 1 FROM events e WHERE e.session = r.parent_session AND e.kind = 'message_appended'
  AND json_extract(e.payload, '$.uuid') = m.uuid
  AND json_extract(e.payload, '$.sender_session_id') = r.child_session);

-- func: remember_text_seq
UPDATE subagent_reports SET delivered_text_seq = @text_seq
WHERE child_session = @child_session AND result_seq = @result_seq AND state = 'pending';

-- func: mark_delivered
UPDATE subagent_reports SET state = 'delivered', error = NULL, delivered_at = @delivered_at,
 delivered_text_seq = @delivered_text_seq
WHERE child_session = @child_session AND result_seq = @result_seq AND state = 'pending';

-- func: mark_failed
UPDATE subagent_reports SET state = 'failed', error = @error, delivered_at = @failed_at
WHERE child_session = @child_session AND result_seq = @result_seq AND state = 'pending';

-- func: supersede_older
UPDATE subagent_reports SET state = 'superseded', delivered_at = @superseded_at
WHERE child_session = @child_session AND result_seq < @newest_seq AND state = 'pending' AND source_kind = 'result';

-- func: supersede_repeats
UPDATE subagent_reports SET state = 'superseded', delivered_at = @superseded_at
WHERE state = 'pending' AND source_kind = 'result'
 AND (SELECT MAX(COALESCE(d.delivered_text_seq,
       CASE WHEN d.source_kind != 'result' THEN d.result_seq END)) FROM subagent_reports d
      WHERE d.child_session = subagent_reports.child_session
       AND (d.state = 'delivered' OR (d.source_kind != 'result' AND d.state = 'pending'))
       AND d.result_seq < subagent_reports.result_seq) IS NOT NULL
 AND NOT EXISTS (
  SELECT 1 FROM events e
  WHERE e.session = subagent_reports.child_session AND e.kind = 'agent_event'
   AND e.seq > (SELECT MAX(COALESCE(d.delivered_text_seq,
       CASE WHEN d.source_kind != 'result' THEN d.result_seq END)) FROM subagent_reports d
      WHERE d.child_session = subagent_reports.child_session
       AND (d.state = 'delivered' OR (d.source_kind != 'result' AND d.state = 'pending'))
       AND d.result_seq < subagent_reports.result_seq)
   AND e.seq < subagent_reports.result_seq
   AND json_extract(e.payload, '$.type') = 'assistant'
   AND EXISTS (
    SELECT 1 FROM json_each(json_extract(e.payload, '$.message.content')) AS block
    WHERE json_extract(block.value, '$.type') = 'text'
     AND length(COALESCE(json_extract(block.value, '$.text'), '')) > 0
   )
 );

-- func: turn_events
SELECT payload FROM events
WHERE session = @child_session AND kind = 'agent_event' AND seq <= @result_seq
 AND seq > COALESCE((SELECT MAX(seq) FROM events
  WHERE session = @child_session AND kind = 'agent_event' AND seq < @result_seq
   AND json_extract(payload, '$.type') = 'result'), 0)
 AND json_extract(payload, '$.type') IN ('assistant', 'result')
ORDER BY seq;
