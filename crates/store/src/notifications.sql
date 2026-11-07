-- func: insert_notice
INSERT INTO notifications (id, session, category, title, message, question_id, state, reason, created_at)
VALUES (@id, @session, @category, @title, @message, @question_id, @state, @reason, @created_at);

-- func: notice
SELECT id, session, category, title, message, question_id, state, reason, created_at FROM notifications WHERE id = @id;

-- func: pending
SELECT id FROM notifications WHERE state = 'pending' ORDER BY created_at LIMIT @limit;

-- func: settle
UPDATE notifications SET state = @state, reason = @reason WHERE id = @id AND state = 'pending';

-- func: subscribe
INSERT INTO push_subscriptions (id, endpoint, p256dh, auth, created_at) VALUES (@id, @endpoint, @p256dh, @auth, @created_at)
ON CONFLICT(id) DO UPDATE SET endpoint = excluded.endpoint, p256dh = excluded.p256dh, auth = excluded.auth;

-- func: subscriptions
SELECT id, endpoint, p256dh, auth, created_at FROM push_subscriptions;

-- func: unsubscribe
DELETE FROM push_subscriptions WHERE id = @id;

-- func: claim_delivery
INSERT OR IGNORE INTO push_deliveries (notification, subscription, state) VALUES (@notification, @subscription, 'claimed');

-- func: finish_delivery
UPDATE push_deliveries SET state = @state WHERE notification = @notification AND subscription = @subscription;

-- func: previous_status
SELECT payload FROM events WHERE session = @session AND kind = 'state_summary' ORDER BY seq DESC LIMIT 1 OFFSET 1;

-- func: turn_end
SELECT kind, payload FROM events WHERE session = @session AND (
 kind IN ('session_stopped', 'driver_error', 'model_error', 'delivery_timeout') OR
 (kind = 'agent_event' AND json_extract(payload, '$.type') = 'result')
) ORDER BY seq DESC LIMIT 1;
