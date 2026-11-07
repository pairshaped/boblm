-- func: append
INSERT INTO session_list_events (session, kind, payload, created_at) VALUES (@session, @kind, @payload, @created_at);

-- func: since
SELECT seq, session, kind, payload, created_at FROM session_list_events WHERE seq > @after ORDER BY seq LIMIT @limit;

-- func: latest_cursor
SELECT COALESCE(MAX(seq), 0) AS cursor FROM session_list_events;

-- func: root_of
SELECT COALESCE(root_session_id, parent_session_id) AS root FROM sessions WHERE id = @session;
