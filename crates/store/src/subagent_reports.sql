-- func: all
SELECT child_session, result_seq, parent_session, state, error, created_at, delivered_at, source_kind, error_message
FROM subagent_reports ORDER BY created_at, child_session, result_seq;

-- func: pending
SELECT child_session, result_seq, parent_session, state, error, created_at, delivered_at, source_kind, error_message
FROM subagent_reports WHERE state = 'pending'
ORDER BY created_at, child_session, result_seq;

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
