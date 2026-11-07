-- Count each model call once. Claude emits one assistant event per content block and
-- repeats the message usage on each, so the final event for a message replaces earlier
-- counts instead of adding to them. Additive for live old runners (ADR 0040).
CREATE TABLE usage_model_call (
 session TEXT NOT NULL,
 call_id TEXT NOT NULL,
 provider TEXT NOT NULL,
 model TEXT NOT NULL,
 day TEXT NOT NULL,
 input_tokens INTEGER NOT NULL DEFAULT 0,
 cache_read_tokens INTEGER NOT NULL DEFAULT 0,
 cache_write_tokens INTEGER NOT NULL DEFAULT 0,
 output_tokens INTEGER NOT NULL DEFAULT 0,
 last_seq INTEGER NOT NULL,
 PRIMARY KEY (session, call_id)
) STRICT;

-- Backfill the per-call rows. Assistant messages use their message ID; Codex token
-- usage notifications use their event sequence because each notification is one call.
INSERT INTO usage_model_call(
 session, call_id, provider, model, day,
 input_tokens, cache_read_tokens, cache_write_tokens, output_tokens, last_seq)
SELECT session, call_id, provider, model, day,
 input_tokens, cache_read_tokens, cache_write_tokens, output_tokens, last_seq
FROM (
 SELECT e.session,
  COALESCE(NULLIF(json_extract(e.payload, '$.message.id'), ''), printf('event:%d', e.seq)) AS call_id,
  s.agent AS provider,
  COALESCE(json_extract(e.payload, '$.message.model'), 'unknown') AS model,
  date(e.created_at / 1000, 'unixepoch') AS day,
  COALESCE(json_extract(e.payload, '$.message.usage.input_tokens'), 0) AS input_tokens,
  COALESCE(json_extract(e.payload, '$.message.usage.cache_read_input_tokens'), 0) AS cache_read_tokens,
  COALESCE(json_extract(e.payload, '$.message.usage.cache_creation_input_tokens'), 0) AS cache_write_tokens,
  COALESCE(json_extract(e.payload, '$.message.usage.output_tokens'), 0) AS output_tokens,
  e.seq AS last_seq,
  ROW_NUMBER() OVER (
   PARTITION BY e.session, COALESCE(NULLIF(json_extract(e.payload, '$.message.id'), ''), printf('event:%d', e.seq))
   ORDER BY e.created_at DESC, e.seq DESC
  ) AS position
 FROM events e
 JOIN sessions s ON s.id = e.session
 WHERE e.kind = 'agent_event'
  AND json_extract(e.payload, '$.type') = 'assistant'
  AND json_type(e.payload, '$.message.usage') = 'object'
)
WHERE position = 1;

INSERT INTO usage_model_call(
 session, call_id, provider, model, day,
 input_tokens, cache_read_tokens, cache_write_tokens, output_tokens, last_seq)
SELECT e.session, printf('event:%d', e.seq), s.agent,
 COALESCE(json_extract(e.payload, '$.model'), 'unknown'),
 date(e.created_at / 1000, 'unixepoch'),
 COALESCE(json_extract(e.payload, '$.input_tokens'), 0),
 COALESCE(json_extract(e.payload, '$.cached_input_tokens'), 0),
 COALESCE(json_extract(e.payload, '$.cache_write_input_tokens'), 0),
 COALESCE(json_extract(e.payload, '$.output_tokens'), 0),
 e.seq
FROM events e
JOIN sessions s ON s.id = e.session
WHERE e.kind = 'agent_event'
 AND json_extract(e.payload, '$.type') = 'token_usage';

-- Rebuild the daily aggregate from deduped calls, correcting any overcount from 032.
DELETE FROM usage_token_daily;
INSERT INTO usage_token_daily(
 provider, model, day, input_tokens, cache_read_tokens, cache_write_tokens, output_tokens, calls)
SELECT provider, model, day,
 SUM(input_tokens), SUM(cache_read_tokens), SUM(cache_write_tokens), SUM(output_tokens), COUNT(*)
FROM usage_model_call
GROUP BY provider, model, day;

DROP TRIGGER IF EXISTS usage_token_after_event;

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
  input_tokens, cache_read_tokens, cache_write_tokens, output_tokens, last_seq)
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
  NEW.seq
 FROM sessions s WHERE s.id = NEW.session
 ON CONFLICT(session, call_id) DO UPDATE SET
  provider = excluded.provider,
  model = excluded.model,
  day = excluded.day,
  input_tokens = excluded.input_tokens,
  cache_read_tokens = excluded.cache_read_tokens,
  cache_write_tokens = excluded.cache_write_tokens,
  output_tokens = excluded.output_tokens,
  last_seq = excluded.last_seq
 WHERE excluded.last_seq >= usage_model_call.last_seq;
END;

CREATE TRIGGER usage_model_call_after_insert AFTER INSERT ON usage_model_call
BEGIN
 INSERT INTO usage_token_daily(
  provider, model, day, input_tokens, cache_read_tokens, cache_write_tokens, output_tokens, calls)
 VALUES(NEW.provider, NEW.model, NEW.day,
  NEW.input_tokens, NEW.cache_read_tokens, NEW.cache_write_tokens, NEW.output_tokens, 1)
 ON CONFLICT(provider, model, day) DO UPDATE SET
  input_tokens = usage_token_daily.input_tokens + excluded.input_tokens,
  cache_read_tokens = usage_token_daily.cache_read_tokens + excluded.cache_read_tokens,
  cache_write_tokens = usage_token_daily.cache_write_tokens + excluded.cache_write_tokens,
  output_tokens = usage_token_daily.output_tokens + excluded.output_tokens,
  calls = usage_token_daily.calls + 1;
END;

CREATE TRIGGER usage_model_call_after_update AFTER UPDATE ON usage_model_call
BEGIN
 UPDATE usage_token_daily SET
  input_tokens = input_tokens - OLD.input_tokens,
  cache_read_tokens = cache_read_tokens - OLD.cache_read_tokens,
  cache_write_tokens = cache_write_tokens - OLD.cache_write_tokens,
  output_tokens = output_tokens - OLD.output_tokens,
  calls = calls - 1
 WHERE provider = OLD.provider AND model = OLD.model AND day = OLD.day;
 DELETE FROM usage_token_daily WHERE calls <= 0;
 INSERT INTO usage_token_daily(
  provider, model, day, input_tokens, cache_read_tokens, cache_write_tokens, output_tokens, calls)
 VALUES(NEW.provider, NEW.model, NEW.day,
  NEW.input_tokens, NEW.cache_read_tokens, NEW.cache_write_tokens, NEW.output_tokens, 1)
 ON CONFLICT(provider, model, day) DO UPDATE SET
  input_tokens = usage_token_daily.input_tokens + excluded.input_tokens,
  cache_read_tokens = usage_token_daily.cache_read_tokens + excluded.cache_read_tokens,
  cache_write_tokens = usage_token_daily.cache_write_tokens + excluded.cache_write_tokens,
  output_tokens = usage_token_daily.output_tokens + excluded.output_tokens,
  calls = usage_token_daily.calls + 1;
END;
