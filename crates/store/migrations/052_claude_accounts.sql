CREATE TABLE claude_account_exhaustion (
    account_key TEXT NOT NULL PRIMARY KEY,
    until INTEGER NOT NULL
);
CREATE TABLE claude_account_handoffs (
    session TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    account_key TEXT NOT NULL,
    until INTEGER NOT NULL,
    PRIMARY KEY(session, account_key)
);
CREATE TABLE claude_account_continuations (
    session TEXT NOT NULL PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE
);
