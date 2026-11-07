-- The sidebar order is stored: projects in one list, and each project's chats in their own.
-- A new item takes the highest position in its list plus one, so it shows at the bottom until
-- the owner drags it. An empty list counts as 0, so the first item is 1.
ALTER TABLE projects ADD COLUMN position INTEGER NOT NULL DEFAULT 0;
UPDATE projects SET position = 1 + (
    SELECT COUNT(*) FROM projects o
    WHERE o.created_at < projects.created_at OR (o.created_at = projects.created_at AND o.id < projects.id)
);
ALTER TABLE sessions ADD COLUMN position INTEGER NOT NULL DEFAULT 0;
UPDATE sessions SET position = 1 + (
    SELECT COUNT(*) FROM sessions o
    WHERE o.project_id = sessions.project_id
      AND (o.created_at < sessions.created_at OR (o.created_at = sessions.created_at AND o.id < sessions.id))
);
CREATE INDEX sessions_by_project_position ON sessions(project_id, position);
