-- Add-only for live runners: record one runaway pause per parent-message round.
CREATE TABLE subagent_round_pauses (
 session TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 round_seq INTEGER NOT NULL,
 tokens INTEGER NOT NULL,
 calls INTEGER NOT NULL,
 max_tokens INTEGER NOT NULL,
 max_calls INTEGER NOT NULL,
 created_at INTEGER NOT NULL,
 PRIMARY KEY (session, round_seq)
) STRICT;
