-- func: create
INSERT INTO sessions (id, name, working_directory, project_id, agent, model, effort, applied_model, applied_effort, created_at, position)
VALUES (@id, @name, @working_directory, @project_id, @agent, @model, @effort, @model, @effort, @created_at,
        (SELECT COALESCE(MAX(position), 0) + 1 FROM sessions WHERE project_id = @project_id));

-- func: list
SELECT id, working_directory, agent_session_id, created_at, archived, model, effort, name, applied_model, applied_effort, project_id, agent, account_id, parent_session_id, root_session_id, role, account_key FROM sessions ORDER BY position, id;

-- func: set_agent_session
UPDATE sessions SET agent_session_id = @agent_session_id WHERE id = @id;

-- func: set_archived
UPDATE sessions SET archived = @archived WHERE id = @id;

-- func: set_model_effort
UPDATE sessions SET model = @model, effort = @effort WHERE id = @id;

-- func: set_name
UPDATE sessions SET name = @name WHERE id = @id;

-- func: set_applied_choice
UPDATE sessions SET applied_model = @model, applied_effort = @effort WHERE id = @id;

-- func: agent_session
SELECT agent_session_id FROM sessions WHERE id = @id;

-- func: set_account
UPDATE sessions SET account_id = @account_id WHERE id = @id;

-- func: set_account_key
UPDATE sessions SET account_key = @account_key WHERE id = @id;

-- func: set_parent_link
UPDATE sessions SET parent_session_id = @parent_session_id, root_session_id = @root_session_id, role = @role WHERE id = @id;

-- func: set_role
UPDATE sessions SET role = @role WHERE id = @id;

-- func: link
SELECT parent_session_id, root_session_id FROM sessions WHERE id = @id;

-- func: parent_session_id
SELECT parent_session_id FROM sessions WHERE id = @id;

-- func: last_peer_sender
SELECT CAST(json_extract(payload, '$.sender_session_id') AS TEXT) AS sender
FROM events
WHERE session = @session AND kind = 'message_appended'
 AND json_type(payload, '$.sender_session_id') = 'text'
 AND CAST(json_extract(payload, '$.sender_session_id') AS TEXT) <> @session
ORDER BY seq DESC LIMIT 1;

-- func: subagents
SELECT id FROM sessions WHERE root_session_id = @root_session_id OR parent_session_id = @root_session_id ORDER BY created_at, id;

-- func: subagent_summary
SELECT
 (SELECT COUNT(*) FROM sessions c
  WHERE (c.root_session_id = @root_session_id OR c.parent_session_id = @root_session_id)
   AND c.archived = 0) AS open,
 (SELECT COUNT(*) FROM questions q JOIN sessions c ON c.id = q.session
  WHERE (c.root_session_id = @root_session_id OR c.parent_session_id = @root_session_id)
   AND q.retired = 0 AND q.answer IS NULL AND q.discuss_later = 0) AS questions,
 (SELECT COUNT(*) FROM sessions c
  WHERE (c.root_session_id = @root_session_id OR c.parent_session_id = @root_session_id)
   AND c.archived = 0
   AND (
    SELECT CASE
     WHEN e.kind = 'agent_event' THEN
      CASE WHEN json_extract(e.payload, '$.type') = 'result'
       AND json_extract(e.payload, '$.is_error') = 1 THEN 1 ELSE 0 END
     ELSE 1 END
    FROM events e
    WHERE e.session = c.id AND (
     e.kind IN ('driver_error', 'model_error', 'delivery_timeout')
     OR (e.kind = 'agent_event' AND json_extract(e.payload, '$.type') = 'result'))
    ORDER BY e.seq DESC LIMIT 1
   ) = 1) AS failures,
 (SELECT COUNT(*) FROM sessions c
  WHERE (c.root_session_id = @root_session_id OR c.parent_session_id = @root_session_id)
   AND c.archived = 0
   AND EXISTS (SELECT 1 FROM events e
    WHERE e.session = c.id AND e.kind = 'subagent_report_failed')) AS report_failures;

-- func: idle_subagents
SELECT s.id, COALESCE(MAX(e.created_at), s.created_at) AS last_activity
FROM sessions s LEFT JOIN events e ON e.session = s.id
WHERE s.root_session_id IS NOT NULL AND s.archived = 0
GROUP BY s.id
HAVING COALESCE(MAX(e.created_at), s.created_at) <= @cutoff
ORDER BY last_activity, s.id;

-- func: prunable_subagents
SELECT s.id FROM sessions s
WHERE (s.parent_session_id IS NOT NULL OR s.root_session_id IS NOT NULL)
 AND s.archived = 1
 AND COALESCE((
  SELECT MAX(e.created_at) FROM events e
  WHERE e.session = s.id AND e.kind = 'archive_changed'
   AND json_extract(e.payload, '$.archived') = 1
 ), s.created_at) <= @cutoff
 AND NOT EXISTS (
  SELECT 1 FROM subagent_reports r
  WHERE r.child_session = s.id AND r.state = 'pending'
 )
ORDER BY s.id;

-- func: has_pending_subagent_report
SELECT EXISTS(
 SELECT 1 FROM subagent_reports WHERE child_session = @child_session AND state = 'pending'
) AS pending;

-- func: last_activity
SELECT COALESCE(MAX(e.created_at), s.created_at) AS last_activity
FROM sessions s LEFT JOIN events e ON e.session = s.id
WHERE s.id = @id
GROUP BY s.id;

-- func: position_in_project
SELECT position FROM sessions WHERE id = @id AND project_id = @project_id;

-- func: set_position
UPDATE sessions SET position = @position WHERE id = @id;

-- func: reopen
UPDATE sessions SET archived = 0 WHERE id = @id AND archived = 1;

-- func: runner_stopped
SELECT stopped FROM runner_state WHERE session = @session;

-- func: set_runner_stopped
INSERT INTO runner_state (session, stopped) VALUES (@session, @stopped)
ON CONFLICT(session) DO UPDATE SET stopped = excluded.stopped;

-- func: runner_quiet_stop
SELECT quiet_stop FROM runner_state WHERE session = @session;

-- func: set_runner_quiet_stop
INSERT INTO runner_state (session, quiet_stop) VALUES (@session, @quiet_stop)
ON CONFLICT(session) DO UPDATE SET quiet_stop = excluded.quiet_stop;
