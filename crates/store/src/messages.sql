-- func: append
INSERT INTO messages (uuid, session, seq, role, content, delivery_state, created_at)
VALUES (@uuid, @session, (SELECT COALESCE(MAX(seq), 0) + 1 FROM messages WHERE session = @session), @role, @content, @delivery_state, @created_at);

-- func: range
SELECT uuid, session, seq, role, content, CASE WHEN expired_at IS NOT NULL THEN 'expired' ELSE delivery_state END AS delivery_state, created_at, retry_count, expired_from FROM messages
WHERE session = @session AND seq >= @start AND seq < @end ORDER BY seq;

-- func: lookup
SELECT uuid, session, seq, role, content, CASE WHEN expired_at IS NOT NULL THEN 'expired' ELSE delivery_state END AS delivery_state, created_at, retry_count, expired_from FROM messages WHERE uuid = @uuid;

-- func: transition
UPDATE messages SET delivery_state = @next WHERE uuid = @uuid AND delivery_state = @expected AND expired_at IS NULL;

-- func: dispatch
UPDATE messages SET delivery_state = 'dispatching'
WHERE uuid = @uuid AND delivery_state = 'queued' AND expired_at IS NULL AND created_at > @cutoff;

-- func: sender
SELECT CAST(json_extract(payload, '$.sender_session_id') AS TEXT) AS sender_session_id
FROM events WHERE session = @session AND kind = 'message_appended'
AND json_extract(payload, '$.uuid') = @uuid LIMIT 1;

-- func: queue_retry
UPDATE messages SET delivery_state = 'queued', retry_count = @retry_count WHERE uuid = @uuid AND delivery_state = 'rejected' AND expired_at IS NULL;

-- func: outstanding
SELECT uuid, session, seq, role, content, CASE WHEN expired_at IS NOT NULL THEN 'expired' ELSE delivery_state END AS delivery_state, created_at, retry_count, expired_from FROM messages
WHERE session = @session AND expired_at IS NULL AND delivery_state IN ('queued', 'dispatching', 'uncertain', 'rejected')
ORDER BY seq;

-- func: human_input
SELECT human_input FROM messages WHERE uuid = @uuid;

-- func: mark_human_input
UPDATE messages SET human_input = 1 WHERE uuid = @uuid;

-- func: expire_stale
UPDATE messages SET expired_at = @now, expired_from = delivery_state
WHERE session = @session AND expired_at IS NULL AND delivery_state IN ('queued', 'uncertain')
AND created_at <= @cutoff
RETURNING uuid, expired_from;
