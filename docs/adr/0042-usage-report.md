# 0042: Token usage and cost report

`usage_report` is a read-only peer tool available to every agent. It answers how
much each model consumed and what it cost, without starting an agent or scanning
the full event history. There is no CLI surface. Arguments are optional `since`
and `until` Toronto dates in `YYYY-MM-DD` form, plus optional exact `model` and
`provider` filters. The default range is the last seven calendar days, inclusive.

## Output

Token usage is grouped by model and by week, with weeks starting on Monday in
Toronto time. Each group reports uncached input tokens, cache read tokens, cache write
tokens, output tokens, cache hit rate, model call count, and, for Pi only, an
estimated cost in US dollars. Cache hit rate is cache reads divided by uncached
input plus cache reads plus cache writes. Claude and Codex run on subscription
plans, so their groups carry token counts and no dollar figure.

Pi cost uses `PI_MODEL_COST_INPUT`, `PI_MODEL_COST_CACHE_READ` and
`PI_MODEL_COST_OUTPUT` from the tuning crate, per million tokens. DeepInfra does
not report a separate cache-write price, so cache writes contribute nothing to
the estimate.

The report also returns one allowance row per Claude or Codex account per day.
Each row is the last weekly-allowance reading recorded that day, with its reset
time, observation time and sample count. `dailyConsumedPercent` is the change
from the previous calendar day when that day exists and the reset window is the
same. It is null when the previous day is missing, the reset changed, or usage
dropped inside the window. That number is the daily share of the weekly plan,
the input to choosing a Sonnet threshold for routing.

## Incremental aggregate

Migration 032 adds `usage_token_daily` and `usage_allowance_daily`. Migration 033
adds `usage_model_call` and rebuilds the token aggregate from it. Both are
purely additive, so live old runners keep inserting their existing event shapes
(ADR 0040). SQLite triggers on `events` update the aggregates as events arrive,
including events from old runners that have not restarted.

Claude Code emits one assistant event per content block and repeats the same
message usage on each event. Summing every event overcounts input, cache and
output. `usage_model_call` is keyed by `(session, call_id)`; assistant events use
the message ID, so a later event for the same message replaces the earlier row.
Codex token usage uses the event sequence as its call ID because each
notification is one model call. Triggers apply the difference to the daily
aggregate when a call row is inserted or replaced. Migration 033's backfill picks
the last assistant event per message before rebuilding the daily table, so
history is deduplicated the same way. Pi normally emits one assistant event per
message; its test asserts that, and the same dedupe applies if a message repeats.

The token triggers read Claude and Pi assistant messages (`message.usage` with
input, output, cache read and cache write counts) and Codex `token_usage` events.
Codex previously stored only context totals from `thread/tokenUsage/updated`, not
the request breakdown. The Codex translator now emits a `token_usage` event with
the model, uncached input, cached input, cache write and output counts. Codex
includes cached input in its input total, so the translator subtracts cached and
cache-write input before storing the uncached count. Its model comes from
`thread/settings/updated`, with the thread's configured model as the initial
value. Reasoning output is already included in output, so it is not added again.
History recorded before this change has no Codex token breakdown and cannot be
reconstructed.

A read-only scan of 717 real Codex rollout files (103,174 `token_usage_record`
entries) found no repeated `response_id`: each model call had one record. Child
and subagent threads do have their own token usage (20,031 records in 390 files,
identified by `parent_thread_id`), but the driver filters notifications to the
parent thread ID before translation. A fake-app-server test sends child and
parent token usage and asserts that only the parent counts reach the event
stream.

The allowance trigger reads weekly readings from usage events and attributes
them through the chat's recorded account ID. It keeps the last reading of each
day, with latest observation wins. A chat with no account ID records no daily
allowance row, because guessing an account would combine unrelated plans.

Migration 044 preserves exact UTC milliseconds on each deduplicated model call,
backfilled from its last event sequence, and records every allowance observation.
Additive triggers capture timestamps even when old runners insert events. The
existing UTC daily aggregates remain for compatibility; the report reads calls
and observations through timestamp indexes instead of those daily aggregates.

`REPORT_TIME_ZONE` selects the IANA zone used by chrono-tz in Rust. Its value is
America/Toronto. Inclusive date arguments become a half-open UTC timestamp range
between local midnights. Day and Monday week grouping, last observation selection,
sample counts, allowance deltas and the default seven calendar days all use that
zone, including daylight saving transitions. The report never scans the event log.

## Tests and limits

Fixture tests cover Claude, Codex and Pi token events, weekly grouping, cost,
filters, invalid dates, allowance deltas across days, and the migration backfill
followed by live trigger updates. The live check calls the tool once against a
`bin/dev-instance`.

Codex token counts are complete only from the release that introduced the
`token_usage` event. Allowance history needs a recorded account ID, which chats
gain when their runner starts on account-aware code or their child launches.

Deleted chats can leave model-call aggregates whose source events are gone. Migration
045 fills their missing timestamps with noon UTC on the recorded UTC day so the
report retains those calls. Noon UTC falls on the same Toronto date throughout
the year. This is an approximation: calls made before Toronto midnight may have
belonged to the previous local date, which cannot be recovered without events.
Exact event timestamps remain unchanged.
