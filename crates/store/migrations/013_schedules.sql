CREATE TABLE schedules (
    id TEXT PRIMARY KEY NOT NULL,
    session TEXT NOT NULL REFERENCES sessions(id),
    prompt TEXT NOT NULL CHECK (length(prompt) > 0),
    arguments TEXT NOT NULL CHECK (json_valid(arguments)),
    interval_ms INTEGER CHECK (interval_ms IS NULL OR interval_ms > 0),
    next_run_at INTEGER NOT NULL,
    ends_at INTEGER,
    max_runs INTEGER CHECK (max_runs IS NULL OR max_runs > 0),
    runs INTEGER NOT NULL DEFAULT 0,
    skipped INTEGER NOT NULL DEFAULT 0,
    last_run_at INTEGER,
    state TEXT NOT NULL CHECK (state IN ('active', 'done', 'missed')),
    created_at INTEGER NOT NULL
) STRICT;
CREATE INDEX schedules_due ON schedules(state, next_run_at);
CREATE INDEX schedules_session ON schedules(session);
