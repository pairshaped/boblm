-- The chat list gains a subagents_changed event: a root's subagent summary changed in a way no
-- other list event shows (a question asked, answered or removed, a report delivery failure).
-- Add-only for live old runners: they still insert the kinds they know, which stay allowed, and
-- the table keeps its name, columns, rows and sequence counter.
CREATE TABLE session_list_events_new (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    session TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('created', 'renamed', 'archived', 'state_summary', 'deleted', 'subagents_changed')),
    payload TEXT NOT NULL CHECK (json_valid(payload)),
    created_at INTEGER NOT NULL
) STRICT;
INSERT INTO session_list_events_new SELECT * FROM session_list_events;
-- AUTOINCREMENT counters live in sqlite_sequence; carry the old one over so a sequence number
-- that a browser has already seen is never issued again after the old table's last rows went.
DELETE FROM sqlite_sequence WHERE name = 'session_list_events_new';
INSERT INTO sqlite_sequence (name, seq)
SELECT 'session_list_events_new', seq FROM sqlite_sequence WHERE name = 'session_list_events';
DROP TABLE session_list_events;
ALTER TABLE session_list_events_new RENAME TO session_list_events;
