-- func: create
INSERT INTO sessions (id, name, working_directory, project_id, agent, model, effort, applied_model, applied_effort, created_at, position)
VALUES (@id, @name, @working_directory, @project_id, @agent, @model, @effort, @model, @effort, @created_at,
        (SELECT COALESCE(MAX(position), 0) + 1 FROM sessions WHERE project_id = @project_id));

-- func: list
SELECT id, working_directory, agent_session_id, created_at, archived, model, effort, name, applied_model, applied_effort, project_id, agent, account_id FROM sessions ORDER BY position, id;

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
