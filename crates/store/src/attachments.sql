-- func: create
INSERT INTO attachments (id, session, name, content_type, size, sha256, path, created_at)
VALUES (@id, @session, @name, @content_type, @size, @sha256, @path, @created_at);

-- func: lookup
SELECT id, session, name, content_type, size, sha256, path, created_at, message_uuid, stored FROM attachments WHERE id = @id;

-- func: for_message
SELECT id, session, name, content_type, size, sha256, path, created_at, message_uuid, stored FROM attachments
WHERE message_uuid = @message_uuid ORDER BY position;

-- func: link
UPDATE attachments SET message_uuid = @message_uuid, position = @position WHERE id = @id AND message_uuid IS NULL;

-- func: orphans
SELECT id, path FROM attachments WHERE message_uuid IS NULL AND created_at < @cutoff;

-- func: delete_orphan
DELETE FROM attachments WHERE id = @id AND message_uuid IS NULL;

-- func: mark_stored
UPDATE attachments SET stored = 1 WHERE id = @id;
