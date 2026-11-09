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
