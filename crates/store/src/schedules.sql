-- func: create
INSERT INTO schedules (id, session, prompt, arguments, interval_ms, next_run_at, ends_at, max_runs, state, created_at)
VALUES (@id, @session, @prompt, @arguments, @interval_ms, @next_run_at, @ends_at, @max_runs, 'active', @created_at);

-- func: lookup
SELECT id, session, prompt, arguments, interval_ms, next_run_at, ends_at, max_runs, runs, skipped, last_run_at, state, created_at
FROM schedules WHERE id = @id;

-- func: for_session
SELECT id, session, prompt, arguments, interval_ms, next_run_at, ends_at, max_runs, runs, skipped, last_run_at, state, created_at
FROM schedules WHERE session = @session ORDER BY created_at, id;

-- func: count_active
SELECT COUNT(*) FROM schedules WHERE session = @session AND state = 'active';

-- func: due
SELECT s.id, s.session, s.prompt, s.arguments, s.interval_ms, s.next_run_at, s.ends_at, s.max_runs, s.runs, s.skipped, s.last_run_at, s.state, s.created_at
FROM schedules s JOIN sessions ON sessions.id = s.session
WHERE s.state = 'active' AND s.next_run_at <= @now AND sessions.archived = 0
ORDER BY s.next_run_at, s.id;

-- func: advance
UPDATE schedules
SET runs = runs + @ran, skipped = skipped + @skipped,
    last_run_at = CASE WHEN @ran = 1 THEN @occurrence ELSE last_run_at END,
    next_run_at = @next_run_at, state = @state
WHERE id = @id AND state = 'active' AND next_run_at = @occurrence;

-- func: delete
DELETE FROM schedules WHERE id = @id AND session = @session;
