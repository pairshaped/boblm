-- func: was_deleted
SELECT id FROM deleted_sessions WHERE id = @id;

-- func: archived
SELECT archived FROM sessions WHERE id = @id;

-- func: subagents
SELECT id FROM sessions WHERE root_session_id = @id OR parent_session_id = @id ORDER BY created_at, id;

-- func: saving_credentials
SELECT COUNT(*) FROM secret_requests WHERE session = @id AND state = 'saving';

-- func: queue_uploads
INSERT OR IGNORE INTO deleted_uploads (path) SELECT path FROM attachments WHERE session = @id;

-- func: attachments
DELETE FROM attachments WHERE session = @id;

-- func: messages
DELETE FROM messages WHERE session = @id;

-- func: events
DELETE FROM events WHERE session = @id;

-- func: tools
DELETE FROM tool_call_names WHERE session = @id;

-- func: schedules
DELETE FROM schedules WHERE session = @id;

-- func: secrets
DELETE FROM secret_requests WHERE session = @id;

-- func: spawns
DELETE FROM peer_spawn_requests WHERE request_id = @id OR caller = @id;

-- func: list_events
DELETE FROM session_list_events WHERE session = @id;

-- func: receipt
INSERT INTO deleted_sessions (id) VALUES (@id);

-- func: session
DELETE FROM sessions WHERE id = @id;

-- func: pending_uploads
SELECT path FROM deleted_uploads;

-- func: upload_removed
DELETE FROM deleted_uploads WHERE path = @path;
