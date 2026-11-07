-- func: measure
WITH reviewers AS (
 SELECT s.id,
  COALESCE((SELECT CAST(json_extract(e.payload, '$.turnActive') AS INTEGER)
   FROM events e WHERE e.session = s.id AND e.kind = 'state_summary'
   ORDER BY e.seq DESC LIMIT 1), 0) AS mid_turn,
  COALESCE((SELECT MAX(e.created_at) FROM events e WHERE e.session = s.id
   AND e.kind IN ('agent_event', 'message_appended')), s.created_at) AS activity_at,
  COALESCE((SELECT MAX(0, c.input_tokens) + MAX(0, c.cache_read_tokens)
   + MAX(0, c.cache_write_tokens) + MAX(0, c.output_tokens)
   FROM usage_model_call c WHERE c.session = s.id AND c.provider = 'codex'
   AND c.model = @model ORDER BY c.last_seq DESC LIMIT 1), 0) AS last_tokens
 FROM sessions s WHERE s.account_key = @account_key AND s.agent = 'codex'
  AND s.role = 'reviewer' AND s.archived = 0 AND s.model = @model
)
SELECT
 CAST(COALESCE((SELECT SUM(MAX(0, c.input_tokens) + MAX(0, c.cache_read_tokens)
  + MAX(0, c.cache_write_tokens) + MAX(0, c.output_tokens))
  FROM usage_model_call c JOIN sessions s ON s.id = c.session
  WHERE c.provider = 'codex' AND s.account_key = @account_key AND c.model = @model
   AND c.created_at > @since AND c.created_at <= @now), 0) AS INTEGER) AS recent_tokens,
 CAST(COALESCE((SELECT SUM(last_tokens) FROM reviewers WHERE mid_turn = 1), 0) AS INTEGER) AS projected_tokens,
 (SELECT COUNT(*) FROM reviewers WHERE mid_turn = 1 OR activity_at > @since) AS active_reviewers;
