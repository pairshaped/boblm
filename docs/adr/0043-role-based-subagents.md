# 0043: Role-based subagents

This supersedes ADR 0015's independent-child rule for role spawns and extends
ADR 0036's drawer. A parent asks BobLM for an implementer, reviewer or planner and never
chooses the model. BobLM resolves the role from a role table in the tuning crate
and the account readings from ADR 0037. The result names the chosen model,
effort, account, step and reason.

A visible root chat can
spawn subagents; a subagent cannot spawn another subagent. At most three open
subagents are allowed per root. Native Claude Agent/Task and Codex multi-agent
use are turned off for new provider launches.

## Roles and efforts

Planners take no difficulty and use Opus high on the Claude plan. They remain
available until the Claude weekly allowance reaches the 2% cutoff, without the Sonnet R
reserve and without a fallback. An exhausted planner request is refused with the
Claude reset time. Missing readings and readings past reset count as available.

planner: only when the owner explicitly asks for a planner. Agents must not spawn
one on their own initiative or because a workflow or skill says to plan. This is
agent guidance, not a server check: BobLM cannot determine who requested it.


Implementers take a difficulty of `simple` or `hard`:

1. Sol on the Codex plan allowance, medium or high.
2. Sol on the Codex plan credits while the balance is positive, medium or high.
3. Sonnet on the Claude plan while Claude's remaining weekly allowance is above
   the days to reset times R, medium or high.
4. DeepSeek V4.1 Flash 1017 through Pi, medium or high.

Reviewers take no difficulty:

1. Luna on the OpenAI API credit while API headroom is available, xhigh.
2. Haiku on the Claude Platform API credit, high.
3. DeepSeek V4.1 Flash 1017 through Pi, high.

Every Claude plan step stops taking new subagents once the account has
CLAUDE_ACCOUNT_CUTOFF_PERCENT (2%) of its weekly allowance left. That last slice is kept
for the chats already running on it; new work falls through to the next step.

The grunt role is retired. Both spawn tools refuse new grunt requests with a plain
sentence directing callers to an implementer or `spawn_peer` with an explicit
model for mechanical work. Stored sessions and receipts retain their role and
remain readable. Reviewers do independent code review; implementers own
implementation and repairs.

`CODEX_PLAN_ROUTING_ENABLED` and `OPENAI_API_ROUTING_ENABLED` are documented
tunables, both true. Disabling the first skips both Sol plan and credits steps;
disabling the second skips Luna API reviews. The route reason names the flag,
for example "disabled by CODEX_PLAN_ROUTING_ENABLED". Drivers, the model catalog
and explicit model choices keep working. There is no settings screen (ADR 0018).

The OpenAI API credit is reserved for Luna reviews. Implementers use the ordered
plan, credits, Claude and DeepSeek chain above.
R starts at 14 percentage points of the Claude weekly plan per day
(`ROLE_R_POINTS_PER_DAY`). It is measured later from `usage_report`. A reading
past its reset counts as available, and a plan account with no reading counts as
available. A step with an
active quota or billing exhaustion mark is skipped until its reset. There are no
BobLM spend budgets; provider limits remain the owner's controls. The generous
per-round runaway backstop below catches subagents that keep going unchecked.

The runner detects a provider quota or billing rejection in a role chat's Pi,
Codex or Claude error envelope and marks that chat's routed step, after checking
that the session's provider account matches the step. An
OpenAI API quota or billing mark lasts until the start of the next OPENAI_API_CREDIT_RESET_DAY (the 1st, the ChatGPT plan renewal date) UTC, or the month's
last day when it is shorter. The Claude API account has $200 of monthly credit;
quota or billing rejection marks Haiku exhausted until the next
`ANTHROPIC_API_CREDIT_RESET_DAY` UTC. That tunable defaults to 1 until the actual
renewal date is known. It uses exhaustion marks, without the Claude plan cutoff
or Sonnet reserve. A step with no known reset, currently DeepSeek, is
retried after `ROLE_EXHAUSTION_RETRY` (one hour). The resolver reads the marks in
the same request that chooses a route, so a marked step is skipped immediately.

A provider error containing "Rate limit reached" and "tokens per min (TPM)" marks
only the API reviewer step for `OPENAI_API_RATE_LIMIT_COOLDOWN` (two minutes),
including when Codex says it will retry. A short cooldown cannot replace a longer
quota or billing mark. Other rate limits leave exhaustion marks alone. If the
turn ends in error after a TPM rejection, the parent report says the review may
be incomplete and suggests using `ping_peer` in a few seconds to continue.
Nothing is resent automatically.

## API reviewer headroom

At spawn time, the store measures Luna calls on `codex-api` in the last
`OPENAI_API_TPM_WINDOW` (60 seconds), including cached input and output tokens.
It adds each unarchived API reviewer's last call size again when that reviewer
is mid-turn, even when the last call is older than the window. The combined
number must be strictly below `OPENAI_API_TPM_LIMIT` (2,000,000) times
`OPENAI_API_TPM_BUDGET` (0.5), a 1,000,000-token spawn budget. The unused half
leaves room for already running reviews to finish as their contexts grow.

The number of unarchived API reviewers must also be strictly below
`OPENAI_API_MAX_ACTIVE_REVIEWERS` (3). A reviewer counts while mid-turn or when
its latest agent event or input is within the same 60-second window. A newly
spawned reviewer counts immediately, before it has a usage sample. Route
selection and receipt creation share a server lock so simultaneous spawns
cannot all claim the last slot. The lock stays held if the caller disconnects.

These defaults retain the owner's requested buffer. A local dev-instance copy
of the usage records contained 983 API Luna calls in the latest day, averaging
107,307 tokens with a maximum of 247,186. Three requests at that maximum reserve
about 742,000 tokens before counting recent completed requests. This is a spawn guard, not a guarantee against a provider rate limit;
long reviews keep their account and may send several requests per minute.
The route reason includes the measured number that blocked an API spawn and
the chosen fallback. Haiku API fallback depends on credit exhaustion marks.

The role table, efforts, R and model IDs are constants in `crates/tuning`. The
resolver is pure: `resolve(role, difficulty, readings, exhausted_marks, api_headroom, now)`
returns a chosen route or an exhausted outcome with one plain-English reason per
step (ADR 0021). It does no I/O and takes `now` explicitly, so every boundary is
table-tested. The Claude cutoff uses fractional days to reset, so it falls
smoothly by the hour rather than stepping by whole days.

## Role spawn tools

`spawn_subagent(role, difficulty, title, message, requestId, workingDirectory)`
creates a hidden subagent under the calling root. Its first message carries the
subagent reply trailer, so the child knows its final reply reaches the parent
automatically and `message_parent` is for mid-task questions only. `spawn_peer` accepts either
explicit `model` and `effort` or `role` and `difficulty`; the role form creates a
visible root chat. Both require a title of 1 to 200 characters, stored on the
child in the spawn transaction and included in the receipt identity. Plain
`spawn_peer` keeps its previous independent-root behavior.

The resolved model, effort, account, step and reason are written to the spawn
receipt. An identical retry returns that stored choice and never re-resolves, even
if readings changed. A conflicting reuse still fails. The result names the route
and lists the caller's open subagents with idle time from their last stored event.

A subagent cannot spawn another subagent. A root may have at most
`MAX_OPEN_SUBAGENTS_PER_ROOT` (3) open subagents; the store checks the limit inside
the spawn transaction. A subagent working folder must be inside the caller's
project after canonicalization and must exist as a directory. Both forms of
`spawn_peer` also require an existing project directory before persisting a new chat.
An identical accepted role spawn retry keeps its receipt even if its folder has since
been deleted; it never resumes the child automatically.

Role tools and configured routing steps are enabled.

## Cutover

`ROLE_SUBAGENTS_ENABLED` is true. Claude launches with its `Agent` and `Task`
tools disallowed. Codex launches with the `multi_agent` and `multi_agent_v2`
features off in the thread config and on the command line, and `ultra` is no
longer offered as a Codex effort. The appended system prompt tells agents to
start helpers with `spawn_subagent`, use role-routed `spawn_peer` for long-lived
visible work, expect results as messages, reuse a subagent with `ping_peer`, and
archive it with `archive_peer` when done. The native subagent history and wake
code stays for chats started before the cutover.

## Parent and lifetime

A chat may record a provider `account_key`. The tuning table
`CODEX_ACCOUNT_HOMES` maps `codex-plan` and `codex-api` to their CODEX_HOME
directories; a null key keeps the inherited environment. The AgentFactory sets
that home on the Codex child and reads the account ID from it. Role spawns set the
key from the chosen route: the Codex plan steps use `codex-plan`, and the OpenAI
API step uses `codex-api`. That home's provider reads the OpenAI key from the
child environment, injected from SOPS per spawn (ADR 0023).

`CLAUDE_ACCOUNT_KEY_API_A` names `claude-api-a`. `CLAUDE_ACCOUNT_HOMES` maps it to
an isolated config directory relative to the instance data directory. The factory
sets `CLAUDE_CONFIG_DIR` on that child and injects `ANTHROPIC_API_KEY` from the
single SOPS key `ANTHROPIC_API_KEY_A` (ADR 0025). It removes inherited OAuth,
auth-token and provider-override variables from API children. Subscription children
never inherit an API key or auth-token override. The API home has no subscription
login, and the API account identity is `claude-api-a`, separate from subscription
usage. Missing credentials fail the spawn; they never select a subscription login.


A subagent session records `parent_session_id`, `root_session_id` and `role` in
the same spawn transaction as the child and its first input. A plain root has all
three null; a role-routed root from `spawn_peer` has its role but no parent or root
link. The fields travel in the session list snapshot and the existing created
payload; no new list event kind is added. The browser hides sessions with a parent
or root link from the sidebar on every device. `list_peers` and `inspect_peer`
expose the link and role so an agent can tell a subagent from a root.

The parent archives its own subagents and may reopen them. Archiving a root
cascades to its subagents, and deleting a root deletes its subagents with the same
deletion receipts and upload cleanup as ADR 0030. The idle
backstop archives after 8 hours; the hourly prune deletes subagents 3 days after archive.
The drawer is a read-only live view with Stop and Archive. Subagent questions,
failures and stalls surface on the root's sidebar entry.

## Superseded reports

A subagent can finish more than one turn before it is quiet, for example when queued
follow-up work starts right after a result. Each result queues its own `subagent_reports`
row, but the parent needs one message: the report is built from the child's latest
assistant text, so two rows would send identical text.

When the scheduler finds several pending rows for one child, it delivers the row for the
newest result and marks the older pending rows `superseded`. A superseded row sends nothing
and is never retried. The rule is applied as soon as the newer row exists, without waiting
for the child to go quiet, because the newest row always carries the final text. A result that
arrives after the child was quiet and a report was delivered is a new report, as before. A
failed newest report is recorded as failed and does not revive the superseded rows.

## Error reports

A subagent's `driver_error`, `model_error` or `delivery_timeout` event queues a
report in the same transaction that stores the event. Error reports are delivered
without consulting the child's status. A child whose process never starts keeps
its input held, so waiting for it to go quiet would hide the failure forever.

The parent message names the subagent, quotes the stored error sentence, and says
to fix the cause and use `ping_peer`, or use `archive_peer` and start another
subagent. It includes the same open-subagent list and parent reply trailer as a
result report. A timeout from a live runner that records no message uses a plain
sentence explaining the missing consumption acknowledgment and held input.

Identical error sentences from one child queue one report until a later
`command_lifecycle` `started` acknowledgment, Codex `system` `turn_started` event,
or result. Resume alone does not reset this deduplication. A different error sentence queues a new report. Error reports
are excluded from result coalescing, so a delivered result cannot suppress a later
error, and newer pending results cannot supersede errors. A bare result after an
error adds no report unless the child wrote new assistant text.

Quota, billing and rate-limit failures emitted as assistant text followed by a
result keep their existing result report. Those events do not queue an error
report. Report delivery uses a UUID derived from the child and source event, so a
web restart cannot accept the same report twice at the parent.

## Runaway subagents

Only subagents have a runaway limit. Root chats, including role-routed visible
roots, are never limited. `SUBAGENT_ROUND_MAX_TOKENS` is 400,000,000 input,
cache-read, cache-write and output tokens combined; `SUBAGENT_ROUND_MAX_CALLS`
is 3,000 model calls. Both tunables are deliberately generous: measured lifetime
maxima were 51.5 million input
and 122,000 output tokens over 215 calls for implementers, and 111 million input
and 1.19 million output tokens over 809 calls for reviewers. A long review can
span several rounds. These ceilings catch runaway work rather than budget normal
runs.

A round starts at the latest `message_appended` event whose authenticated sender
is the subagent's parent, including the spawn message. A fresh parent ping starts
a new round immediately. Retrying an accepted message UUID adds no event and
does not reset the round. Owner messages, other peers, results and resume actions
do not start rounds. Usage comes from `usage_model_call` rows after that event
sequence, which orders events even when timestamps match. Repeated Claude
content blocks update the same call rather than count as new calls.

After recording and applying a batch containing model usage, the runner checks
the round. Equality is allowed; crossing either ceiling records one row in
`subagent_round_pauses`, a `driver_error` event and its error report atomically.
The row stores the round sequence, both totals and both limits; the event records
the plain-English reason using the existing error contract. The report's source
kind is `subagent_runaway_limit`. That durable round marker prevents duplicate
interrupts and reports, including after a restart. The serialized actor then uses
its ordinary interrupt path, holding unconfirmed input without discarding or
replaying it. A crash after the marker stops the provider through runner
supervision and recovers with input held.

The runner owns this check so a parent ping cannot change rounds between checking
usage and interrupting the turn. It also works while the web server is down. Live
runners keep their loaded code and acquire the check on their next safe restart;
the round-marker table is an add-only migration and the runner protocol stays
compatible (ADR 0040).

The outbox delivers the pause reason immediately, with the totals, crossed limit,
open-subagent list and report reply trailer. It asks the parent to inspect the
subagent with `inspect_peer` and judge whether it is still making real progress.
After a runaway pause, the parent decides whether to ping for a fresh round or
archive; if unsure, it asks the owner with `ask` before continuing.
Nothing continues automatically.

## Migration compatibility

The parent-link and role-exhaustion migrations are add-only: nullable columns,
an index, and one new table, with no table rebuild or CHECK change. A test runs
the previous release's sessions SQL after migrating. Older live runners keep
their loaded code and continue reading the session rows they already understand
(ADR 0040).

The report-state migration rebuilds the small `subagent_reports` table to allow the
`superseded` state and recreates the queue trigger unchanged in the same transaction. Rows
and columns are kept, and a runner that has not restarted still queues `pending` rows
through the trigger.

The error-report migration adds two columns and a new event trigger. Existing
result inserts default to `source_kind = 'result'`; the result trigger and existing
columns remain compatible with loaded runners. Error text is stored separately
from a failure to deliver the report, so delivery bookkeeping cannot erase it.

## Root idle nudges

A consumed report can trigger one idle-nudge opportunity for its root parent
(ADR 0046). Its authenticated child sender and derived report message UUID identify
it. Direct peer messages do not qualify. Open children and pending report rows
participate in the quiet check; subagents never receive idle nudges themselves.
