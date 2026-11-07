-- A project is a base directory; chats belong to one and work in its directory.
CREATE TABLE projects (
    id TEXT PRIMARY KEY NOT NULL,
    path TEXT NOT NULL UNIQUE CHECK (length(path) > 0),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    created_at INTEGER NOT NULL
) STRICT;
ALTER TABLE sessions ADD COLUMN project_id TEXT REFERENCES projects(id);
ALTER TABLE sessions ADD COLUMN agent TEXT NOT NULL DEFAULT 'claude' CHECK (agent IN ('claude', 'codex'));
-- The resume ID is the agent's own, whichever agent it is.
ALTER TABLE sessions RENAME COLUMN claude_session_id TO agent_session_id;

-- Existing chats get a project per distinct directory. A trailing slash names the same folder.
INSERT INTO projects (id, path, name, created_at)
SELECT lower(substr(h, 1, 8) || '-' || substr(h, 9, 4) || '-4' || substr(h, 14, 3) || '-8' || substr(h, 18, 3) || '-' || substr(h, 21, 12)),
       path,
       CASE WHEN base = '' THEN substr(path, 1, 200) ELSE substr(base, 1, 200) END,
       first_seen
FROM (
    SELECT hex(randomblob(16)) AS h,
           folder AS path,
           replace(folder, rtrim(folder, replace(folder, '/', '')), '') AS base,
           MIN(created_at) AS first_seen
    FROM (SELECT CASE WHEN rtrim(working_directory, '/') = '' THEN '/' ELSE rtrim(working_directory, '/') END AS folder, created_at FROM sessions)
    GROUP BY folder
);
UPDATE sessions SET project_id = (
    SELECT id FROM projects
    WHERE projects.path = CASE WHEN rtrim(sessions.working_directory, '/') = '' THEN '/' ELSE rtrim(sessions.working_directory, '/') END
);

-- Every chat belongs to a project. SQLite cannot add a NOT NULL column over existing rows
-- without rebuilding the table, so the database refuses a chat without one instead.
CREATE TRIGGER sessions_need_a_project_on_insert BEFORE INSERT ON sessions WHEN NEW.project_id IS NULL
BEGIN SELECT RAISE(ABORT, 'a chat must belong to a project'); END;
CREATE TRIGGER sessions_need_a_project_on_update BEFORE UPDATE OF project_id ON sessions WHEN NEW.project_id IS NULL
BEGIN SELECT RAISE(ABORT, 'a chat must belong to a project'); END;

-- The stored event kinds say "agent" now that two agents produce them.
UPDATE events SET kind = 'agent_event' WHERE kind = 'claude_event';
UPDATE events
SET kind = 'agent_session_changed',
    payload = json_remove(json_set(payload, '$.agent_session_id', json_extract(payload, '$.claude_session_id')), '$.claude_session_id')
WHERE kind = 'claude_session_changed';
