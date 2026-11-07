CREATE TABLE events (
    session TEXT NOT NULL REFERENCES sessions(id),
    seq INTEGER NOT NULL CHECK (seq > 0),
    kind TEXT NOT NULL,
    payload TEXT NOT NULL CHECK (json_valid(payload)),
    created_at INTEGER NOT NULL,
    PRIMARY KEY (session, seq)
) STRICT;
