# 0004: Session execution and delivery

One session task owns one serialized command queue and drives a provider driver
for Claude, Codex or Pi.
Callers share its handle. Commands from multiple devices resolve in queue
arrival order. Synchronous SQLite operations run on Tokio blocking workers;
mutations and their durable events commit before process writes or replies.
Client UUIDs are canonicalized (lowercase, hyphenated) at every entry point
before comparison, storage and replies, so one message has one spelling. Client
UUIDs are checked before dispatch. Identical retries do not execute
again, whether or not the session is archived, and conflicting content under the
same UUID fails. A fresh owner prompt reopens an archived session in the same
transaction that accepts the message and claims its attachments. Failed input
and identical retries leave the archive state unchanged. Peer and scheduled
prompts cannot reopen archived sessions.

The send receipt confirms durable acceptance. The actor hands off that receipt
before dispatching the accepted input, so its provider startup, configuration,
or input write does not delay the receipt. Commands still enter the serialized
queue, so earlier actor work can delay acceptance. Only a lifecycle acknowledgment
proves consumption.
The dispatcher's cached inputs contain queued, dispatching, uncertain, and rejected
messages. Consumed history stays in storage and is queried by UUID when validating
an explicit retry.

## Recovery

On task startup, unresolved dispatching messages become uncertain. They remain
held and are never replayed. Queued input remains queued without starting
Claude. A new owner message or explicit resume releases the queued input. A scheduled
prompt also releases the hold when no other input is queued and the owner has
not stopped the chat. This check counts queued input only, so an uncertain
message does not prevent the release; uncertain input itself is never replayed.
Status exposes queued count, process-running state, queueHeld, and delivery
counts. A held queue can be resumed even when the Claude process is idle.
The agent's own session ID is persisted from init and supplied on resume;
failed resume never silently starts a fresh conversation.

## Expiry

A queued, held or uncertain message older than QUEUED_MESSAGE_MAX_AGE (6 hours) is stale and
expires: a terminal delivery state, `expired`, that is never sent. Expiry never resends anything,
so the never-replay rule holds for uncertain input too. The runner owns delivery transitions, so
it owns the sweep: once at startup after unresolved dispatches become uncertain, again when the
oldest expirable message comes due (rechecked at least every QUEUED_MESSAGE_SWEEP_INTERVAL), and
as a guard immediately before a dispatch. The store expires only rows still queued or uncertain
whose age exceeds the limit, in one transaction that also appends a `delivery_changed` event with
`expired_from`. A dispatching or consumed message is never touched, and a dispatch transition
from queued requires the row to be unexpired and newer than the cutoff, checked inside the
transition itself so a message that ages out during driver startup is expired instead of sent.
A message cannot be both.

`delivery_state` has a CHECK constraint that admits no new value without rebuilding a table live
runners write. Expiry is therefore stored as nullable `expired_at` and `expired_from` columns
(add-only, ADR 0040). Readers report an expired row as delivery state `expired`, and the
dispatcher's working set excludes it. Expiry clears the hold only when no queued or uncertain
message remains and the owner has not stopped the chat; a fresh message behind an expired one
still waits for a new owner message or Resume. A runner started before this change does not sweep
and would still see an expired row as queued, so a chat's runner must be replaced before its
queue is trusted to expire. The sender of an expired peer message gets no notice. Rolling a runner back to code from before migration 043 is unsafe once any row has been expired, because the old dispatch path ignores `expired_at`.

The chat keeps an expired message in place, marked with a note that it expired unsent after the
limit; an expired unconfirmed message adds that it may already have been received. Expired
messages leave the queued and uncertain counts, so the sidebar count drops them, and they are not
in the uncertain banner. The sidebar counts a held queue as `held`, not `queued`. While a
queue is held by a driver error, status carries that error as `heldReason`, and the chat shows it
beside Resume. The reason is runner memory only, cleared when the hold lifts.

## Consumption and steering

A stream write is not consumption. Either started or completed for a message
UUID proves consumption, including completion after a result. Cancellation,
discard, or refusal rejects only unconsumed dispatching input. Cancellation
never requeues consumed input. Active-turn owner messages use priority now. Claude receives those messages with
`origin: {kind: "human"}`, allowing eligible foreground tools to move into the
background. Peer and automated messages use Claude priority next without human
origin. Other providers retain their steering contract. Human provenance is
recorded atomically with acceptance and survives queue recovery and reconnects;
identical retries cannot change it. A queued acknowledgment for Claude next input
pauses its consumption deadline while already-consumed work is active. A turn
result starts the bounded post-result deadline; queued alone never proves
consumption. Legacy input without provenance is treated
as automated. Explicit Stop uses the interrupt control request.

The injected prompt asks agents to prefer native background execution for work
expected to take more than about half a second, return control promptly, and
use completion notifications. Dependencies and shared-resource locks still apply.
Human-origin now can still interrupt writing or tools Claude cannot background.
While an agent is compacting, all new owner, peer, and scheduled messages remain
queued. Ready driver events are applied before outbound dispatch, so a queued
compaction marker wins over concurrently accepted input. A dispatched `/compact`
reserves this guard before its start notification arrives. Provider compaction-start/status signals also set it. An explicit
compaction boundary, cleared compaction status, turn result, command refusal,
or process disconnect releases the guard. A normal turn-start signal extends
acknowledgment deadlines without blocking steering. Explicit Stop and Archive
remain available during compaction.

Slash commands wait until idle and until every follow-up acknowledgment has
settled. A cutover result does not release a waiting slash while follow-ups
remain unacknowledged. Claude Code 2.1.308 emits a result for every slash command, including `/cost`
and `/compact` (docs/live-slash-commands.md), so a result always ends the turn. A late started acknowledgment, including one for input already held as
uncertain, makes the session active;
a late completed acknowledgment after its result permits idle.

## Rejection and deadlines

Steering deadlines are provider-aware. Pi and Codex consume a steer at the next
tool boundary, so a steer dispatched into an active turn pauses its clock while a
tool call is outstanding. Claude initially keeps the ordinary deadline. Once any
provider reports lifecycle `queued`, the message also waits for a boundary and its
clock pauses during outstanding tool calls. This applies to Claude priority now
and next input. A Claude steer without that acknowledgment keeps its deadline.
The clock restarts when the provider reports the last outstanding tool result.
A steer that arrives as the turn ends becomes the next turn's input, without a
second write. The first turn result gives queued input a fresh
ACK_DEADLINE_AFTER_RESULT window, including after a long tool call. Later results
never extend that window. Only lifecycle `started` or `completed` proves
consumption. Pausing never replays uncertain input: a provider that stays silent
after the boundary is still stopped at the deadline.

Rejection, its requeue or its delivery_rejected event commit in one
transaction. Rejected input returns to queued in its original sequence position. It retries
automatically at idle, at most twice. Retry counts persist across restarts.
After those retries fail it stays rejected and emits a delivery_rejected event
with Retry available. Explicit retry resets the automatic retry count. It is
available for rejected messages; uncertain input is never replayed.

The acknowledgment deadline starts at dispatch with ACK_DEADLINE (60 seconds). The first
result shortens the remaining window to at most ACK_DEADLINE_AFTER_RESULT (30 seconds),
except queued input waiting for a boundary gets the fresh window described above. Later results
never extend it. On expiry the process stops and unresolved input becomes
uncertain. The queue remains held for the next owner message or explicit resume.

## Stop and archive

Stop interrupts through the driver's control channel. An interrupt acknowledgment
is never final. Stop completes on the interrupted turn's result or process
exit, regardless of whether the control reply arrives before or after it.
The result wait is bounded at STOP_RESULT_DEADLINE (30 seconds), then the process group is killed.
During this wait the task continues accepting commands and persists new input
as queued. It cannot dispatch a new turn before the interrupted result is
handled. Status exposes stopping separately from turn activity. Unacknowledged input
becomes uncertain and queued input remains held. Archive interrupts any active
turn, stops the process, and preserves queued input, history, resume ID, model,
and effort. Reopen starts nothing. A new owner message or explicit resume can
release queued input after reopen, as after restart.

## Recording events

Every Claude event is stored before it is applied. The session task takes the
event it woke for plus every event the driver already has queued, up to MAX_EVENT_BATCH (256), and
commits them in one transaction; it then applies
each in order. The database runs in WAL mode with `synchronous=NORMAL`: a commit
survives an application crash or a killed runner, and only an operating-system
crash or power loss can drop the last few commits, which the delivery state
machine recovers as uncertain. This keeps an fsync off every commit, so one chat's
write does not stall the other chats' writers. A sparse stream commits per event, and a burst shares one commit, so
storage keeps pace with streaming output without weakening durability.
`crates/runner/tests/throughput.rs` measures it (about 11,000 events/s, against
about 150 when each event had its own commit).

## Reading events

Event pages read SQLite on blocking workers without entering the session command
queue. Read-only chat lists and event pages use the actor's latest published
status snapshot. The actor publishes queued state before replying to a send and
dispatching state before writing to the child. The serialized status command
remains available as a barrier for callers that need earlier commands to settle.
Status and events can advance between reads; each event cursor still covers only
committed records.

Events are durable per-session pages addressed by the last received sequence.
A consumer always requests events strictly after its cursor. Runner does not
broadcast into an unbounded per-device queue. A slow consumer catches up from
SQLite, so transitioning from catch-up to another read cannot lose an event.
One session task must be owned for each session ID by the application supervisor.

Browser streaming uses HTTP Server-Sent Events served from cursor_page. Each
open session resumes strictly after its Last-Event-ID sequence. Reads and
cancelled subscriptions change no delivery state. ADR 0005 specifies the
command bodies and event framing.


## Process lifetime

Production actors run in independently supervised per-chat processes (ADR 0040).
The HTTP process uses a socket-backed SessionHandle and detaches on shutdown.
Tests can inject drivers into a local actor with the same interface. Stop is
persisted in runner_state; a replacement actor restores it while holding queued
input and recovering unacknowledged dispatches as uncertain. Reconnecting the web
process to a live actor performs none of that recovery and never resets its state.

## Idle nudge delivery

The server may accept one automated idle nudge after an automatically woken root
turn (ADR 0046). Its acceptance transaction preserves human provenance as false.
The duplicate automated-send receipt refreshes the runner queue without clearing
Stop, releasing a hold or reopening a chat. Ordinary consumption acknowledgments
and recovery rules apply.
