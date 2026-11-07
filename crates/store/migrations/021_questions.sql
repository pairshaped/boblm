CREATE TABLE questions (
 id TEXT NOT NULL PRIMARY KEY,
 session TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 question TEXT NOT NULL,
 choices TEXT NOT NULL,
 answer TEXT,
 created_at INTEGER NOT NULL,
 answered_at INTEGER,
 delivery_accepted INTEGER NOT NULL DEFAULT 0 CHECK (delivery_accepted IN (0, 1))
);
CREATE INDEX questions_session ON questions(session, created_at);
