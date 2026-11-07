-- A chat's request for the owner to save a credential into a SOPS file. The credential itself
-- is never stored, here or in an event; only where it should go.
CREATE TABLE secret_requests (
    id TEXT PRIMARY KEY NOT NULL,
    session TEXT NOT NULL REFERENCES sessions(id),
    file TEXT NOT NULL CHECK (length(file) > 0),
    key TEXT NOT NULL CHECK (json_valid(key)),
    state TEXT NOT NULL CHECK (state IN ('pending', 'saving', 'saved', 'canceled', 'expired', 'failed')),
    expires_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;
CREATE INDEX secret_requests_open ON secret_requests(state, expires_at);
CREATE INDEX secret_requests_session ON secret_requests(session);
