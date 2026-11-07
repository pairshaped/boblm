# 0022: Scheduled and recurring prompts

A chat can have prompts that BobLM sends to its agent later: once at a given time, or
repeatedly at a fixed interval (a keep-awake nudge). They are managed only through an API,
which agents use for themselves; there is no screen, composer button or list for them
(ADR 0009). The API is three peer tools (ADR 0015), always bound to the calling session:
`schedule_prompt`, `list_schedules` and `cancel_schedule`.

## Creating

`schedule_prompt` takes a fresh `scheduleId` UUID, the `prompt`, and when to run: `at`
(an RFC 3339 timestamp) or `inMinutes` for the first run, and `everyMinutes` to repeat.
A repeating schedule that names no first run starts one interval from now. `endAt` and
`maxRuns` stop a repeating schedule. The same `scheduleId` with the same arguments returns
the existing schedule, even if a one-shot's time has passed since the first call; different
arguments under the same ID fail, so a retry is safe.
Rejected with a message that says why: a first run in the past or more than
MAX_SCHEDULE_HORIZON (a year) away, an interval under MIN_SCHEDULE_INTERVAL (one minute),
an empty or over-long prompt, or more than MAX_SCHEDULES_PER_CHAT (20) active schedules in
the chat. Times are stored as UTC milliseconds and shown as RFC 3339 UTC.

`list_schedules` returns the chat's schedules with their next run, interval, end, run and
skip counts, last run and state (`active`, `done` or `missed`). `cancel_schedule` deletes
one. Settling a due occurrence and cancelling share one lock and the scheduler rechecks the
schedule under it, so a cancel that succeeds means the occurrence was not sent. Finished schedules stay listed until cancelled, so an agent can see what happened.

## Firing

One scheduler task in the server wakes every SCHEDULER_TICK and sends each due prompt as an
automated message headed "Scheduled prompt:" for a one-shot, or
"Scheduled prompt (every 2 hours):" for a repeating prompt, with its actual interval so the chat and the agent can both
see where it came from. It never steers a running turn: like a slash command it waits for
idle. The message ID is derived from the schedule ID and the occurrence time, so a crash
between sending and recording cannot send it twice (ADR 0011). The runner releases the hold only when the scheduled prompt is the sole queued
input and the owner has not stopped the chat (ADR 0004).

A repeating schedule skips an occurrence that comes due while its chat is busy (a turn
running or stopping, input queued or being delivered, or the owner having stopped it) and
counts it as skipped, so nudges never pile up. A one-shot is queued whatever the chat is
doing, so it is not lost. An agent process that is not running is started by a firing, as by
an owner message.

Two rules keep a nudge from overriding the owner. A scheduled prompt lifts a chat's held
queue (ADR 0004) only when it is the sole queued input; a restart holds an empty queue, so
a nudge still wakes such a chat, but input queued before the restart stays held and the
prompt queues behind it. The release check counts queued input only: uncertain
input does not prevent release, though it is never replayed. And after the owner presses Stop, the chat is left alone, repeating
runs are skipped and a one-shot waits queued, until the owner sends a message, resumes or
retries, or reopens the chat.

## Downtime, archive and missed runs

Archived chats are not scheduled. When the server starts, or a chat is reopened, anything
overdue is settled before firing resumes: a repeating schedule that missed its time
(by more than SCHEDULE_ON_TIME_SLACK) continues at its next future occurrence and does not
catch up; a one-shot at most SCHEDULE_MISSED_GRACE (one hour) overdue fires once, and an
older one is marked `missed` and not sent. A repeating schedule ends as `done` after its
last run (`maxRuns`, or its next run falling after `endAt`).

## Storage and tests

Schedules are a table with their run history counters. A firing updates the schedule and
sends the message in an order that makes a crash harmless in either direction: the message
is idempotent, and recording a run only moves the schedule forward. Tests drive the
scheduler with an explicit clock against a fake driver: one-shot and repeating firing,
idempotent creation, limits and their messages, busy skipping, one-shot queueing, held
queues, archive and reopen, restart settling, the end conditions, and the tools' ownership
checks.

## Automatic idle nudges

The same server scheduler checks one idle-nudge opportunity after a root turn
started by an automated wake (ADR 0046). These are separate durable opportunities,
not user schedules. Scheduled prompts and idle nudges cannot trigger another idle
nudge. Both use deterministic message UUIDs and the automated delivery path.
