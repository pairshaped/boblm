CREATE TABLE sessions (
    id TEXT PRIMARY KEY NOT NULL,
    working_directory TEXT NOT NULL,
    claude_session_id TEXT,
    created_at INTEGER NOT NULL
) STRICT;
CREATE TABLE messages (
    uuid TEXT PRIMARY KEY NOT NULL,
    session TEXT NOT NULL REFERENCES sessions(id),
    seq INTEGER NOT NULL CHECK (seq > 0),
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'system')),
    content TEXT NOT NULL,
    delivery_state TEXT NOT NULL CHECK (delivery_state IN ('queued', 'dispatching', 'started', 'rejected', 'uncertain')),
    created_at INTEGER NOT NULL,
    UNIQUE (session, seq)
) STRICT;
