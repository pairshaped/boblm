-- func: create
INSERT INTO secret_requests (id, session, file, key, state, expires_at, created_at)
VALUES (@id, @session, @file, @key, 'pending', @expires_at, @created_at);

-- func: lookup
SELECT id, session, file, key, state, expires_at, created_at FROM secret_requests WHERE id = @id;

-- func: count_open
SELECT COUNT(*) FROM secret_requests WHERE session = @session AND state IN ('pending', 'saving') AND expires_at > @now;

-- func: claim
UPDATE secret_requests SET state = 'saving' WHERE id = @id AND state = 'pending' AND expires_at > @now;

-- func: settle
UPDATE secret_requests SET state = @state WHERE id = @id AND state = @from_state;

-- func: stale_pending
SELECT id, session, file, key, state, expires_at, created_at FROM secret_requests WHERE state = 'pending' AND expires_at <= @now;

-- func: stale_saving
SELECT id, session, file, key, state, expires_at, created_at FROM secret_requests WHERE state = 'saving' AND expires_at <= @cutoff;
