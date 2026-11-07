CREATE TABLE peer_spawn_requests (
    request_id TEXT PRIMARY KEY NOT NULL REFERENCES sessions(id),
    caller TEXT NOT NULL REFERENCES sessions(id),
    arguments TEXT NOT NULL CHECK (json_valid(arguments))
) STRICT;
