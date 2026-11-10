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
INSERT INTO deleted_sessions (id, deleted_by_session_id) VALUES (@id, @deleted_by_session_id);

-- func: session
DELETE FROM sessions WHERE id = @id;

-- func: pending_uploads
SELECT path FROM deleted_uploads;

-- func: upload_removed
DELETE FROM deleted_uploads WHERE path = @path;

-- func: agent_receipt
SELECT request_id, caller, session_id_prefix, session_id FROM peer_delete_requests WHERE request_id = @request_id;

-- func: save_agent_receipt
INSERT INTO peer_delete_requests (request_id, caller, session_id_prefix, session_id)
VALUES (@request_id, @caller, @session_id_prefix, @session_id);

-- func: project_targets
SELECT target.id, target.archived FROM sessions target JOIN sessions caller ON caller.id = @caller
WHERE caller.project_id IS NOT NULL AND target.project_id = caller.project_id
ORDER BY target.id;

-- func: pending_questions
SELECT COUNT(*) FROM questions WHERE session = @id AND retired = 0 AND answer IS NULL AND discuss_later = 0;

-- func: deletion_actor
SELECT deleted_by_session_id FROM deleted_sessions WHERE id = @id;
