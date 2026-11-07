# 0046: One idle nudge after an automated wake

BobLM gives a root chat one chance to continue when an automatically woken turn
ends and leaves nothing that will wake it again. The server scheduler owns the
timer. A database trigger records new root result events from existing runners,
so enabling the nudge needs no runner replacement or transport change.
Subagent results do not create nudge opportunities: their reports wake the parent,
which decides what to do.

## Turn origin

The first fresh consumption acknowledgment between the previous result and this
result identifies the input that started the turn. A `started` or `completed`
lifecycle event proves consumption; acceptance and writes do not. Repeated
acknowledgments for an input consumed before the previous result are ignored.
A later automated steer cannot turn an owner-started turn into a nudge trigger.
Assistant output or tool-result activity before origin evidence makes the origin
inconclusive and suppresses the nudge.

For Codex, BobLM ignores a `user` event before origin evidence only when its
nonempty content array consists entirely of `tool_result` blocks with nonempty
string `tool_use_id` values. Each ID must match an assistant `tool_use` issued
before the previous result, with no matching call issued between that result and
the user event. These late results belong to earlier turns, so a background
command finishing can still identify the next turn through its job notice's
consumption acknowledgment. Unknown IDs, malformed blocks, mixed content and
current-turn calls remain inconclusive. Claude keeps the conservative rule;
the observed late-result pattern is specific to Codex.

An eligible origin is one of:

- A runner-injected background command completion. Its authenticated sender is
  the chat itself and its content has the runner's completion prefix (ADR 0038).
- A BobLM subagent report. Its authenticated sender and message UUID must match
  the durable report's child and source event (ADRs 0036 and 0043). A direct peer
  message, including a mid-task message from a child, does not qualify.
- Claude's native `system` event with subtype `task_notification`, when it is the
  first origin evidence after the preceding result. The driver preserves this
  subtype even though it discards notification fields it does not understand.
- A runner-injected legacy Codex native subagent completion, identified by its
  authenticated self sender and subagent notice prefix.

Codex background completions and BobLM reports use message acknowledgments.
Claude background completions use the native subtype; BobLM reports use message
acknowledgments. Pi supports acknowledged BobLM reports but exposes no background
job completion or native self-wake through its driver.

Owner, ordinary peer, scheduled and nudge inputs do not qualify. Missing origin
information suppresses the nudge. Native provider delivery that emits no preserved
notification or acknowledgment is therefore weaker: BobLM leaves it alone. This
also excludes a completion acknowledged only after its result, because it cannot
prove the origin of that ended turn. The implementation does not infer work from
assistant prose or turn-end wording.

## Quiet check and acceptance

At `IDLE_NUDGE_DELAY` (60 seconds), subject to the ordinary scheduler tick, BobLM
checks the root's serialized status. It requires no active or stopping turn,
queued, dispatching, uncertain, rejected or held input, Stop, or archive. A running
provider must supply an available job inventory. A listed job blocks the nudge
only while its first-seen time is within `IDLE_NUDGE_RECENT_JOB_WINDOW` (30 minutes)
of the quiet check, including the exact window boundary. Older jobs count as
long-lived and do not supply a future wake for this check. A long-running preview
server therefore stops suppressing nudges; a recent build alongside it still
blocks. A build that outlasts the window can receive the ordinary one-shot nudge.
Without a provider process there are no jobs. Missing or unavailable inventories
still suppress the nudge.

BobLM stores first-seen times in `idle_nudge_jobs`, keyed by chat and job ID.
An event trigger records the first observation in an inventory event or actor
summary. Repeated inventories do not renew the window. A job that disappears and
reappears with the same ID keeps its original time until its chat is deleted.
These ages apply only to idle nudges. Subagent report checks, the runner's quiet
stop and job-completion notices keep their ordinary job rules.

Open children must have no active or stopping turn, queued or dispatching input,
or recent listed jobs. An idle child held while waiting for its parent does not
supply a future wake and does not block a nudge. Unanswered `ask` questions anywhere
under the root and pending child reports suppress the nudge. New input or turn
activity after the triggering result suppresses it, even when that intervening
work has already ended. Suppression settles this opportunity permanently.

Acceptance takes the SQLite write lock and rechecks delivery, Stop, archive,
questions, pending reports and newer activity. It checks the latest durable actor
summaries for the root and its open children as well, using the same job-age rule
and quiet-check time under the write lock. Activity that can change quiet state
and is not reflected in a summary makes that snapshot inconclusive. Control
replies, usage samples, repeated empty inventories and available inventories with
the same job list as the summary do not invalidate a quiet snapshot. Changed
inventories still make an unpublished snapshot inconclusive, even if every listed
job is old. The root's triggering result is already covered by the serialized
status read. This guards against owner input, a new
child, a job update or a child starting work during the scheduler's earlier reads.
Retained native Agent/Task and Codex SubagentActivity lifecycle events also guard
legacy children that have no BobLM child-session row. Incomplete or unknown native
lifecycle state suppresses the opportunity.

The same transaction appends an automated user-role message with human provenance
false and settles the opportunity as queued. The server then calls the ordinary
`send_automated` path with that identical UUID and text. Its duplicate receipt
refreshes the actor's queue and dispatches through normal consumption tracking.
Because the message already exists, this handoff cannot clear a later Stop,
release a hold, reopen a chat or change its provenance.

`IDLE_NUDGE_TEXT` is:

> Your last turn ended but nothing is running that will wake you. If work remains, continue it now. If you're finished, or waiting on the owner or a peer, say so in one line and end your turn.

## Restart and delivery

Each opportunity is keyed by chat and result-event sequence. Its message UUID is
derived from those values with a dedicated hash namespace, like scheduled prompt
occurrences (ADR 0022). Acceptance and settlement commit together. A crash before
acceptance leaves a pending opportunity; a crash after acceptance leaves the same
queued message for handoff. A crash after handoff can repeat the duplicate receipt
but cannot create a second message or repeat consumed input.

A web restart preserves live runner state and durable first-seen times. The job-age
migration seeds the latest inventory of each chat at migration time, so an
existing job with no recorded age blocks for a full window. Unknown ages always
block; a later inventory event or actor summary records the first observation
and starts its window. Loaded runners feed the trigger through their existing
event writes. If the runner itself restarts, its ordinary recovery holds queued
input and uncertain dispatches (ADR 0004). The
nudge does not release that hold. No past results are backfilled at migration.
A turn started by the nudge has no eligible origin and cannot create a chain.

The migrations add durable opportunities and job observations with event triggers.
Loaded runners continue writing their existing SQL. Opportunities and job
observations disappear with their chat. Fake-driver tests cover origins, quiet
blockers, job ages in roots and children, mixed inventories, restart behavior,
one-shot timing, automated provenance, chain prevention and interrupted
acceptance or handoff.
