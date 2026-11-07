-- Keep job ages independent of loaded runner versions and web process restarts.
CREATE TABLE idle_nudge_jobs (
 session TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 job_id TEXT NOT NULL,
 first_seen INTEGER NOT NULL,
 PRIMARY KEY(session, job_id)
) STRICT;

-- Existing inventories have no known first-seen time. Give them a full window
-- from migration rather than inferring their age from historical events.
INSERT INTO idle_nudge_jobs(session, job_id, first_seen)
SELECT e.session, CAST(json_extract(job.value, '$.id') AS TEXT),
 CAST(unixepoch('subsec') * 1000 AS INTEGER)
FROM sessions s JOIN events e ON e.session = s.id
 AND e.seq = (SELECT MAX(latest.seq) FROM events latest WHERE latest.session = s.id
  AND (latest.kind = 'state_summary' OR (latest.kind = 'agent_event'
   AND json_extract(latest.payload, '$.type') = 'background_jobs')))
JOIN json_each(e.payload,
 CASE WHEN e.kind = 'state_summary' THEN '$.jobs.tasks' ELSE '$.snapshot.tasks' END) job
WHERE json_type(job.value, '$.id') = 'text'
ON CONFLICT(session, job_id) DO NOTHING;

-- Summaries also cover jobs recovered from a runner without a fresh inventory event.
-- Keep the first time if an ID disappears and reappears in the same chat.
CREATE TRIGGER observe_idle_nudge_jobs AFTER INSERT ON events
WHEN NEW.kind = 'state_summary' OR (NEW.kind = 'agent_event'
 AND json_extract(NEW.payload, '$.type') = 'background_jobs')
BEGIN
 INSERT INTO idle_nudge_jobs(session, job_id, first_seen)
 SELECT NEW.session, CAST(json_extract(job.value, '$.id') AS TEXT), NEW.created_at
 FROM json_each(NEW.payload,
  CASE WHEN NEW.kind = 'state_summary' THEN '$.jobs.tasks' ELSE '$.snapshot.tasks' END) job
 WHERE json_type(job.value, '$.id') = 'text'
 ON CONFLICT(session, job_id) DO NOTHING;
END;
