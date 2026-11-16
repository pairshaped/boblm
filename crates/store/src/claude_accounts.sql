-- func: exhaustions
SELECT account_key, until FROM claude_account_exhaustion;

-- func: exhaust
INSERT INTO claude_account_exhaustion(account_key, until) VALUES (@account_key, @until)
ON CONFLICT(account_key) DO UPDATE SET until = MAX(until, excluded.until);

-- func: handoffs
SELECT account_key, until FROM claude_account_handoffs WHERE session = @session;

-- func: record_handoff
INSERT INTO claude_account_handoffs(session, account_key, until) VALUES (@session, @account_key, @until)
ON CONFLICT(session, account_key) DO UPDATE SET until = MAX(until, excluded.until);

-- func: limited
INSERT INTO claude_account_continuations(session) VALUES (@session) ON CONFLICT DO NOTHING;

-- func: needs_continuation
SELECT session FROM claude_account_continuations WHERE session = @session;

-- func: clear_continuation
DELETE FROM claude_account_continuations WHERE session = @session;

-- func: move_guard
SELECT s.archived AS archived, s.account_key AS account_key,
 (SELECT COUNT(*) FROM messages m WHERE m.session = s.id AND m.expired_at IS NULL
   AND m.delivery_state IN ('queued', 'dispatching', 'uncertain')) AS pending
FROM sessions s WHERE s.id = @session;
