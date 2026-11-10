-- Preserve pending reports and give recent jobs a durable scheduler deadline.
ALTER TABLE subagent_reports ADD COLUMN recheck_at INTEGER;
ALTER TABLE subagent_reports ADD COLUMN recheck_jobs TEXT;
CREATE INDEX subagent_reports_recheck ON subagent_reports(state, recheck_at, created_at);

-- A changed inventory can release a report before its age deadline. Repeated
-- inventories with the same job IDs leave it deferred, including after restart.
CREATE TRIGGER recheck_subagent_reports_on_jobs AFTER INSERT ON events
WHEN NEW.kind = 'state_summary' OR (NEW.kind = 'agent_event'
 AND json_extract(NEW.payload, '$.type') = 'background_jobs')
BEGIN
 UPDATE subagent_reports SET recheck_at = NULL, recheck_jobs = NULL
 WHERE child_session = NEW.session AND state = 'pending' AND recheck_at IS NOT NULL
 AND (
  json_extract(NEW.payload, CASE WHEN NEW.kind = 'state_summary'
   THEN '$.jobs.available' ELSE '$.snapshot.available' END) IS NOT 1
  OR json_array_length(NEW.payload, CASE WHEN NEW.kind = 'state_summary'
   THEN '$.jobs.tasks' ELSE '$.snapshot.tasks' END) IS NOT json_array_length(recheck_jobs, '$.tasks')
  OR EXISTS (
   SELECT 1 FROM json_each(NEW.payload, CASE WHEN NEW.kind = 'state_summary'
    THEN '$.jobs.tasks' ELSE '$.snapshot.tasks' END) job
   WHERE NOT EXISTS (SELECT 1 FROM json_each(recheck_jobs, '$.tasks') previous
    WHERE json_extract(previous.value, '$.id') = json_extract(job.value, '$.id'))
  )
 );
END;
