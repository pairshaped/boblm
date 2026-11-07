-- func: create
INSERT INTO questions (id, session, question, choices, created_at, position)
VALUES (@id, @session, @question, @choices, @created_at, (SELECT COALESCE(MAX(position), 0) + 1 FROM questions WHERE session = @session));

-- func: lookup
SELECT id, session, question, choices, answer, created_at, answered_at, delivery_accepted, discuss_later, deferred_at FROM questions WHERE id = @id AND retired = 0;

-- func: owner_session
SELECT session FROM questions WHERE id = @id;

-- func: list
SELECT id, session, question, choices, answer, created_at, answered_at, delivery_accepted, discuss_later, deferred_at FROM questions
WHERE session = @session AND retired = 0 AND (answer IS NOT NULL OR discuss_later = 1 OR id = (
 SELECT id FROM questions WHERE session = @session AND retired = 0 AND answer IS NULL AND discuss_later = 0 ORDER BY position LIMIT 1
))
ORDER BY CASE WHEN answer IS NULL AND discuss_later = 0 THEN 0 WHEN discuss_later = 1 THEN 1 ELSE 2 END, position DESC LIMIT @limit;

-- func: answer
UPDATE questions SET answer = @answer, answered_at = @now, discuss_later = 0 WHERE id = @id AND answer IS NULL;

-- func: defer
UPDATE questions SET discuss_later = 1, deferred_at = @now WHERE id = @id AND answer IS NULL AND discuss_later = 0;

-- func: pending_answers
SELECT id, session, question, choices, answer, created_at, answered_at, delivery_accepted, discuss_later, deferred_at FROM questions q
WHERE retired = 0 AND delivery_accepted = 0 AND (
 answer IS NOT NULL OR (discuss_later = 1 AND NOT EXISTS (
  SELECT 1 FROM questions p WHERE p.session = q.session AND p.retired = 0 AND p.answer IS NULL AND p.discuss_later = 0
 ))) ORDER BY discuss_later, COALESCE(answered_at, deferred_at), position;

-- func: accepted
UPDATE questions SET delivery_accepted = 1 WHERE id = @id;

-- func: list_for_root
SELECT q.id, q.session, q.question, q.choices, q.answer, q.created_at, q.answered_at, q.delivery_accepted, q.discuss_later, q.deferred_at FROM questions q
JOIN sessions s ON s.id = q.session
WHERE (s.id = @root_session_id OR s.root_session_id = @root_session_id OR s.parent_session_id = @root_session_id)
 AND q.retired = 0 AND (q.answer IS NOT NULL OR q.discuss_later = 1 OR q.id = (
 SELECT q2.id FROM questions q2 JOIN sessions s2 ON s2.id = q2.session
 WHERE (s2.id = @root_session_id OR s2.root_session_id = @root_session_id OR s2.parent_session_id = @root_session_id)
  AND q2.retired = 0 AND q2.answer IS NULL AND q2.discuss_later = 0 ORDER BY q2.position LIMIT 1
))
ORDER BY CASE WHEN q.answer IS NULL AND q.discuss_later = 0 THEN 0 WHEN q.discuss_later = 1 THEN 1 ELSE 2 END, q.position DESC, q.id LIMIT @limit;

-- func: count_pending
SELECT COUNT(*) FROM questions WHERE session = @session AND retired = 0 AND answer IS NULL AND discuss_later = 0;

-- func: count_pending_for_root
SELECT COUNT(*) FROM questions q JOIN sessions s ON s.id = q.session
WHERE (s.id = @root_session_id OR s.root_session_id = @root_session_id OR s.parent_session_id = @root_session_id)
 AND q.retired = 0 AND q.answer IS NULL AND q.discuss_later = 0;

-- func: edit_state
SELECT session, question, choices, retired, replacement_of FROM questions WHERE id = @id;

-- func: replace
INSERT INTO questions (id, session, question, choices, created_at, position, replacement_of)
SELECT @id, session, @question, @choices, @created_at, position, id
FROM questions WHERE id = @previous AND retired = 0;

-- func: retire
UPDATE questions SET retired = 1, delivery_accepted = 1 WHERE id = @id;

-- func: suppress_retired_notices
UPDATE notifications SET state = 'suppressed', reason = 'The question was removed or replaced.'
WHERE question_id = @id AND state = 'pending';
