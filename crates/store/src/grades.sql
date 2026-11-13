-- func: subject
SELECT id, name, role, agent, model, effort, account_key, parent_session_id, root_session_id
FROM sessions WHERE id = @id;

-- func: receipt
SELECT caller, resolved FROM peer_spawn_requests WHERE request_id = @request_id;

-- func: existing
SELECT created_at FROM subagent_grades WHERE session = @session;

-- func: save
INSERT INTO subagent_grades (session, grader, rating, note, role, agent, model, effort, step, account_key, title, created_at, updated_at)
VALUES (@session, @grader, @rating, @note, @role, @agent, @model, @effort, @step, @account_key, @title, @now, @now)
ON CONFLICT(session) DO UPDATE SET
 grader = excluded.grader,
 rating = excluded.rating,
 note = excluded.note,
 role = excluded.role,
 agent = excluded.agent,
 model = excluded.model,
 effort = excluded.effort,
 step = excluded.step,
 account_key = excluded.account_key,
 title = excluded.title,
 updated_at = excluded.updated_at;

-- func: one
SELECT session, grader, rating, note, role, agent, model, effort, step, account_key, title, created_at, updated_at
FROM subagent_grades WHERE session = @session;
