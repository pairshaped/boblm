-- Temporary tuning data: a parent's quick grade of a subagent's result (ADR 0048).
-- No foreign keys: a grade outlives the chat it describes, so deleting a chat keeps it.
-- The model, effort, step and account are copied at grading time so the row stays meaningful.
CREATE TABLE subagent_grades (
    session TEXT PRIMARY KEY NOT NULL,
    grader TEXT NOT NULL,
    rating TEXT NOT NULL CHECK (rating IN ('good', 'ok', 'poor')),
    note TEXT,
    role TEXT,
    agent TEXT,
    model TEXT,
    effort TEXT,
    step TEXT,
    account_key TEXT,
    title TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;
