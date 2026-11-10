-- Agent attribution survives removal of the chat and its events.
ALTER TABLE deleted_sessions ADD COLUMN deleted_by_session_id TEXT;
-- Retry identity outlives both the deleted target and the requesting chat.
CREATE TABLE peer_delete_requests (
    request_id TEXT PRIMARY KEY NOT NULL,
    caller TEXT NOT NULL,
    session_id_prefix TEXT NOT NULL,
    session_id TEXT NOT NULL
) STRICT;
