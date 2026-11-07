-- Provider quota or billing marks for role routing steps. Add-only for live old runners.
CREATE TABLE role_step_exhaustion (
 step TEXT PRIMARY KEY NOT NULL,
 until INTEGER NOT NULL,
 marked_at INTEGER NOT NULL,
 reason TEXT NOT NULL
) STRICT;
