-- Incremental token and allowance aggregates for usage_report. Additive so live old
-- runners keep inserting their existing event shapes (ADR 0040).
CREATE TABLE usage_token_daily (
 provider TEXT NOT NULL,
 model TEXT NOT NULL,
 day TEXT NOT NULL,
 input_tokens INTEGER NOT NULL DEFAULT 0,
 cache_read_tokens INTEGER NOT NULL DEFAULT 0,
 cache_write_tokens INTEGER NOT NULL DEFAULT 0,
 output_tokens INTEGER NOT NULL DEFAULT 0,
 calls INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY (provider, model, day)
) STRICT;

CREATE TABLE usage_allowance_daily (
 provider TEXT NOT NULL,
 account_id TEXT NOT NULL,
 day TEXT NOT NULL,
 weekly_used REAL NOT NULL,
 weekly_resets_at INTEGER,
 observed_at INTEGER NOT NULL,
 samples INTEGER NOT NULL DEFAULT 1,
 PRIMARY KEY (provider, account_id, day)
) STRICT;

CREATE TRIGGER usage_token_after_event AFTER INSERT ON events
WHEN NEW.kind = 'agent_event'
 AND (
  (json_extract(NEW.payload, '$.type') = 'assistant'
   AND json_type(NEW.payload, '$.message.usage') = 'object')
  OR json_extract(NEW.payload, '$.type') = 'token_usage'
 )
BEGIN
 INSERT INTO usage_token_daily(
  provider, model, day, input_tokens, cache_read_tokens, cache_write_tokens, output_tokens, calls)
 SELECT s.agent,
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
  1
 FROM sessions s WHERE s.id = NEW.session
 ON CONFLICT(provider, model, day) DO UPDATE SET
  input_tokens = usage_token_daily.input_tokens + excluded.input_tokens,
  cache_read_tokens = usage_token_daily.cache_read_tokens + excluded.cache_read_tokens,
  cache_write_tokens = usage_token_daily.cache_write_tokens + excluded.cache_write_tokens,
  output_tokens = usage_token_daily.output_tokens + excluded.output_tokens,
  calls = usage_token_daily.calls + excluded.calls;
END;

CREATE TRIGGER usage_allowance_after_event AFTER INSERT ON events
WHEN NEW.kind = 'agent_event'
 AND json_extract(NEW.payload, '$.type') = 'usage'
 AND json_type(NEW.payload, '$.weekly') = 'object'
 AND json_extract(NEW.payload, '$.weekly.usedPercent') IS NOT NULL
BEGIN
 INSERT INTO usage_allowance_daily(
  provider, account_id, day, weekly_used, weekly_resets_at, observed_at, samples)
 SELECT s.agent, s.account_id, date(NEW.created_at / 1000, 'unixepoch'),
  json_extract(NEW.payload, '$.weekly.usedPercent'),
  json_extract(NEW.payload, '$.weekly.resetsAt'),
  NEW.created_at, 1
 FROM sessions s
 WHERE s.id = NEW.session
  AND s.account_id IS NOT NULL
  AND s.agent IN ('claude', 'codex')
 ON CONFLICT(provider, account_id, day) DO UPDATE SET
  weekly_used = CASE WHEN excluded.observed_at >= usage_allowance_daily.observed_at
   THEN excluded.weekly_used ELSE usage_allowance_daily.weekly_used END,
  weekly_resets_at = CASE WHEN excluded.observed_at >= usage_allowance_daily.observed_at
   THEN excluded.weekly_resets_at ELSE usage_allowance_daily.weekly_resets_at END,
  observed_at = MAX(usage_allowance_daily.observed_at, excluded.observed_at),
  samples = usage_allowance_daily.samples + 1;
END;

-- One-time backfill. The triggers are already installed, but this writes the aggregates
-- directly so the old events are counted exactly once.
INSERT INTO usage_token_daily(
 provider, model, day, input_tokens, cache_read_tokens, cache_write_tokens, output_tokens, calls)
SELECT s.agent,
 COALESCE(json_extract(e.payload, '$.model'), json_extract(e.payload, '$.message.model'), 'unknown'),
 date(e.created_at / 1000, 'unixepoch'),
 SUM(CASE WHEN json_extract(e.payload, '$.type') = 'token_usage'
  THEN COALESCE(json_extract(e.payload, '$.input_tokens'), 0)
  ELSE COALESCE(json_extract(e.payload, '$.message.usage.input_tokens'), 0) END),
 SUM(CASE WHEN json_extract(e.payload, '$.type') = 'token_usage'
  THEN COALESCE(json_extract(e.payload, '$.cached_input_tokens'), 0)
  ELSE COALESCE(json_extract(e.payload, '$.message.usage.cache_read_input_tokens'), 0) END),
 SUM(CASE WHEN json_extract(e.payload, '$.type') = 'token_usage'
  THEN COALESCE(json_extract(e.payload, '$.cache_write_input_tokens'), 0)
  ELSE COALESCE(json_extract(e.payload, '$.message.usage.cache_creation_input_tokens'), 0) END),
 SUM(CASE WHEN json_extract(e.payload, '$.type') = 'token_usage'
  THEN COALESCE(json_extract(e.payload, '$.output_tokens'), 0)
  ELSE COALESCE(json_extract(e.payload, '$.message.usage.output_tokens'), 0) END),
 COUNT(*)
FROM events e
JOIN sessions s ON s.id = e.session
WHERE e.kind = 'agent_event'
 AND (
  (json_extract(e.payload, '$.type') = 'assistant'
   AND json_type(e.payload, '$.message.usage') = 'object')
  OR json_extract(e.payload, '$.type') = 'token_usage'
 )
GROUP BY 1, 2, 3;

INSERT INTO usage_allowance_daily(
 provider, account_id, day, weekly_used, weekly_resets_at, observed_at, samples)
SELECT provider, account_id, day, weekly_used, weekly_resets_at, observed_at, samples
FROM (
 SELECT s.agent AS provider, s.account_id AS account_id,
  date(e.created_at / 1000, 'unixepoch') AS day,
  json_extract(e.payload, '$.weekly.usedPercent') AS weekly_used,
  json_extract(e.payload, '$.weekly.resetsAt') AS weekly_resets_at,
  e.created_at AS observed_at,
  COUNT(*) OVER (
   PARTITION BY s.agent, s.account_id, date(e.created_at / 1000, 'unixepoch')
  ) AS samples,
  ROW_NUMBER() OVER (
   PARTITION BY s.agent, s.account_id, date(e.created_at / 1000, 'unixepoch')
   ORDER BY e.created_at DESC, e.seq DESC
  ) AS position
 FROM events e
 JOIN sessions s ON s.id = e.session
 WHERE e.kind = 'agent_event'
  AND json_extract(e.payload, '$.type') = 'usage'
  AND json_type(e.payload, '$.weekly') = 'object'
  AND json_extract(e.payload, '$.weekly.usedPercent') IS NOT NULL
  AND s.account_id IS NOT NULL
  AND s.agent IN ('claude', 'codex')
)
WHERE position = 1;
