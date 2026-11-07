-- Deletion receipts outlive chats so delayed requests cannot recreate their IDs.
CREATE TABLE deleted_sessions (id TEXT PRIMARY KEY NOT NULL) STRICT;
-- File removal is retried after a crash without retaining chat content.
CREATE TABLE deleted_uploads (path TEXT PRIMARY KEY NOT NULL) STRICT;
CREATE TABLE session_list_events_new (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    session TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('created', 'renamed', 'archived', 'state_summary', 'deleted')),
    payload TEXT NOT NULL CHECK (json_valid(payload)),
    created_at INTEGER NOT NULL
) STRICT;
INSERT INTO session_list_events_new SELECT * FROM session_list_events;
DROP TABLE session_list_events;
ALTER TABLE session_list_events_new RENAME TO session_list_events;
