CREATE TABLE session_list_events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    session TEXT NOT NULL REFERENCES sessions(id),
    kind TEXT NOT NULL CHECK (kind IN ('created', 'renamed', 'archived', 'state_summary')),
    payload TEXT NOT NULL CHECK (json_valid(payload)),
    created_at INTEGER NOT NULL
) STRICT;
