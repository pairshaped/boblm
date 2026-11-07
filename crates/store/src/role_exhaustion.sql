-- func: all
SELECT step, until, marked_at, reason FROM role_step_exhaustion ORDER BY step;

-- func: mark
INSERT INTO role_step_exhaustion(step, until, marked_at, reason)
VALUES (@step, @until, @marked_at, @reason)
ON CONFLICT(step) DO UPDATE SET
 until = excluded.until,
 marked_at = excluded.marked_at,
 reason = excluded.reason
WHERE excluded.until >= role_step_exhaustion.until;
