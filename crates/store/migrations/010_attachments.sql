CREATE TABLE attachments (
    id TEXT PRIMARY KEY NOT NULL,
    session TEXT NOT NULL REFERENCES sessions(id),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 255),
    content_type TEXT NOT NULL CHECK (length(content_type) BETWEEN 1 AND 255),
    size INTEGER NOT NULL CHECK (size >= 0),
    sha256 TEXT NOT NULL CHECK (length(sha256) = 64),
    path TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    message_uuid TEXT REFERENCES messages(uuid),
    position INTEGER CHECK (position IS NULL OR position >= 0)
) STRICT;
CREATE INDEX attachments_message ON attachments(message_uuid, position);
