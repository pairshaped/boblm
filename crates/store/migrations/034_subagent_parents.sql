-- Role subagents record their parent, root and role. Add-only for live old runners.
ALTER TABLE sessions ADD COLUMN parent_session_id TEXT;
ALTER TABLE sessions ADD COLUMN root_session_id TEXT;
ALTER TABLE sessions ADD COLUMN role TEXT;

CREATE INDEX sessions_root_session_id ON sessions(root_session_id);
CREATE INDEX sessions_parent_session_id ON sessions(parent_session_id);
