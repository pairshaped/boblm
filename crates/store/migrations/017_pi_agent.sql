-- Replace only this column's constraint, preserving all other columns and relationships.
ALTER TABLE sessions ADD COLUMN pi_agent TEXT NOT NULL DEFAULT 'claude' CHECK (pi_agent IN ('claude', 'codex', 'pi'));
UPDATE sessions SET pi_agent = agent;
ALTER TABLE sessions DROP COLUMN agent;
ALTER TABLE sessions RENAME COLUMN pi_agent TO agent;
