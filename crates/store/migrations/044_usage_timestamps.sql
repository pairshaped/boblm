-- Preserve exact timestamps for report-time IANA calendar grouping. Existing UTC
-- aggregates and their triggers remain compatible with live runners (ADR 0040).
ALTER TABLE usage_model_call ADD COLUMN created_at INTEGER;
UPDATE usage_model_call SET created_at = (
 SELECT e.created_at FROM events e
 WHERE e.session = usage_model_call.session AND e.seq = usage_model_call.last_seq
);
CREATE INDEX usage_model_call_time ON usage_model_call(created_at);
-- Include timestamps in the existing call upsert so aggregate triggers see one write.
DROP TRIGGER usage_model_call_after_event;
CREATE TRIGGER usage_model_call_after_event AFTER INSERT ON events
WHEN NEW.kind = 'agent_event'
 AND (
  (json_extract(NEW.payload, '$.type') = 'assistant'
   AND json_type(NEW.payload, '$.message.usage') = 'object')
  OR json_extract(NEW.payload, '$.type') = 'token_usage'
 )
BEGIN
 INSERT INTO usage_model_call(
  session, call_id, provider, model, day,
  input_tokens, cache_read_tokens, cache_write_tokens, output_tokens, last_seq, created_at)
 SELECT NEW.session,
  CASE WHEN json_extract(NEW.payload, '$.type') = 'assistant'
   THEN COALESCE(NULLIF(json_extract(NEW.payload, '$.message.id'), ''), printf('event:%d', NEW.seq))
   ELSE printf('event:%d', NEW.seq) END,
  s.agent,
  COALESCE(json_extract(NEW.payload, '$.model'), json_extract(NEW.payload, '$.message.model'), 'unknown'),
  date(NEW.created_at / 1000, 'unixepoch'),
  CASE WHEN json_extract(NEW.payload, '$.type') = 'token_usage'
   THEN COALESCE(json_extract(NEW.payload, '$.input_tokens'), 0)
   ELSE COALESCE(json_extract(NEW.payload, '$.message.usage.input_tokens'), 0) END,
  CASE WHEN json_extract(NEW.payload, '$.type') = 'token_usage'
   THEN COALESCE(json_extract(NEW.payload, '$.cached_input_tokens'), 0)
   ELSE COALESCE(json_extract(NEW.payload, '$.message.usage.cache_read_input_tokens'), 0) END,
  CASE WHEN json_extract(NEW.payload, '$.type') = 'token_usage'
   THEN COALESCE(json_extract(NEW.payload, '$.cache_write_input_tokens'), 0)
   ELSE COALESCE(json_extract(NEW.payload, '$.message.usage.cache_creation_input_tokens'), 0) END,
  CASE WHEN json_extract(NEW.payload, '$.type') = 'token_usage'
   THEN COALESCE(json_extract(NEW.payload, '$.output_tokens'), 0)
   ELSE COALESCE(json_extract(NEW.payload, '$.message.usage.output_tokens'), 0) END,
  NEW.seq, NEW.created_at
 FROM sessions s WHERE s.id = NEW.session
 ON CONFLICT(session, call_id) DO UPDATE SET
  provider = excluded.provider,
  model = excluded.model,
  day = excluded.day,
  input_tokens = excluded.input_tokens,
  cache_read_tokens = excluded.cache_read_tokens,
  cache_write_tokens = excluded.cache_write_tokens,
  output_tokens = excluded.output_tokens,
  last_seq = excluded.last_seq,
  created_at = excluded.created_at
 WHERE excluded.last_seq >= usage_model_call.last_seq;
END;

CREATE TABLE usage_allowance_observation (
 session TEXT NOT NULL,
 seq INTEGER NOT NULL,
 provider TEXT NOT NULL,
 account_id TEXT NOT NULL,
 weekly_used REAL NOT NULL,
 weekly_resets_at INTEGER,
 observed_at INTEGER NOT NULL,
 PRIMARY KEY (session, seq)
) STRICT;
CREATE INDEX usage_allowance_observation_time ON usage_allowance_observation(observed_at);
CREATE TRIGGER usage_allowance_observation_after_event AFTER INSERT ON events
WHEN NEW.kind = 'agent_event'
 AND json_extract(NEW.payload, '$.type') = 'usage'
 AND json_type(NEW.payload, '$.weekly') = 'object'
 AND json_extract(NEW.payload, '$.weekly.usedPercent') IS NOT NULL
BEGIN
 INSERT INTO usage_allowance_observation
 SELECT NEW.session, NEW.seq, s.agent, s.account_id,
  json_extract(NEW.payload, '$.weekly.usedPercent'),
  json_extract(NEW.payload, '$.weekly.resetsAt'), NEW.created_at
 FROM sessions s WHERE s.id = NEW.session AND s.account_id IS NOT NULL
 AND s.agent IN ('claude', 'codex');
END;
INSERT INTO usage_allowance_observation
SELECT e.session, e.seq, s.agent, s.account_id,
 json_extract(e.payload, '$.weekly.usedPercent'),
 json_extract(e.payload, '$.weekly.resetsAt'), e.created_at
FROM events e JOIN sessions s ON s.id = e.session
WHERE e.kind = 'agent_event' AND json_extract(e.payload, '$.type') = 'usage'
 AND json_type(e.payload, '$.weekly') = 'object'
 AND json_extract(e.payload, '$.weekly.usedPercent') IS NOT NULL
 AND s.account_id IS NOT NULL AND s.agent IN ('claude', 'codex');
