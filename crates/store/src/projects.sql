-- func: create
INSERT INTO projects (id, path, name, created_at, position)
VALUES (@id, @path, @name, @created_at, (SELECT COALESCE(MAX(position), 0) + 1 FROM projects));

-- func: lookup
SELECT id, path, name, created_at FROM projects WHERE id = @id;

-- func: by_path
SELECT id, path, name, created_at FROM projects WHERE path = @path;

-- func: list
SELECT id, path, name, created_at FROM projects ORDER BY position, id;

-- func: chat_count
SELECT COUNT(*) FROM sessions WHERE project_id = @project_id;

-- func: delete
DELETE FROM projects WHERE id = @id;

-- func: position_of
SELECT position FROM projects WHERE id = @id;

-- func: set_position
UPDATE projects SET position = @position WHERE id = @id;

-- func: rename
UPDATE projects SET name = @name WHERE id = @id;
