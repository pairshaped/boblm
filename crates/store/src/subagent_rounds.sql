-- func: over_limit
-- Materialize the boundary once, rather than rescanning event history per call.
WITH round AS MATERIALIZED (
 SELECT s.name, s.parent_session_id,
  (SELECT MAX(e.seq) FROM events e WHERE e.session = s.id
   AND e.kind = 'message_appended'
   AND json_extract(e.payload, '$.sender_session_id') = s.parent_session_id) AS round_seq
 FROM sessions s WHERE s.id = @session AND s.parent_session_id IS NOT NULL
)
SELECT round.name, round.parent_session_id, CAST(COALESCE(round.round_seq, 0) AS INTEGER) AS round_seq,
 CAST(COALESCE(SUM(c.input_tokens + c.cache_read_tokens + c.cache_write_tokens + c.output_tokens), 0) AS INTEGER) AS tokens,
 COUNT(*) AS calls
FROM round JOIN usage_model_call c ON c.session = @session AND c.last_seq > round.round_seq
WHERE NOT EXISTS (
 SELECT 1 FROM subagent_round_pauses p WHERE p.session = @session AND p.round_seq = round.round_seq
)
GROUP BY round.name, round.parent_session_id, round.round_seq;

-- func: pause_round
INSERT INTO subagent_round_pauses(session, round_seq, tokens, calls, max_tokens, max_calls, created_at)
VALUES (@session, @round_seq, @tokens, @calls, @max_tokens, @max_calls, @created_at);

-- func: queue_report
INSERT INTO subagent_reports(child_session, result_seq, parent_session, state, created_at, source_kind, error_message)
SELECT session, seq, @parent_session, 'pending', created_at, 'subagent_runaway_limit', json_extract(payload, '$.message')
FROM events WHERE session = @session AND seq = (SELECT MAX(seq) FROM events WHERE session = @session)
ON CONFLICT(child_session, result_seq) DO UPDATE SET source_kind = excluded.source_kind;
