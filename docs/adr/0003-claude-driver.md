# 0003: Claude process ownership

The driver owns one Claude process in a new Unix process group. Explicit process shutdown (`stop`)
kills the group and waits for the direct child. Drop kills the group and aborts
the reader task; Tokio's child handle arranges direct-child reaping. Descendants
that create a separate process group are outside this guarantee.

The stdout reader also watches for process exit, so a descendant holding stdout
open cannot keep pending delivery acknowledgments alive after Claude exits. While
the event channel is full and unread, pending acknowledgments wait for the reader
to drain rather than settle early, because an unread `started` line may still prove
consumption.
Output is framed as raw bytes, so invalid UTF-8 or a line over MAX_CLAUDE_LINE_BYTES (16 MiB; reported
as an unknown event) is skipped without ending the session; only EOF or a real
read error ends it. After the process exits the reader kills its group and keeps
reading buffered output (waiting at most DRAIN_AFTER_EXIT, two seconds, when nothing is ready,
and never more than DRAIN_LIMIT, five seconds, in total, so an escaped descendant
that keeps writing cannot hold it), so nothing Claude wrote before exiting is lost. Runner bounds the
driver shutdown at DRIVER_SHUTDOWN_TIMEOUT (ten seconds). A full event channel parks the
reader; stop closes the receiver first, so shutdown never waits on an unread
channel. Unknown event types do not terminate the reader. Partial
stream events retain their JSON payload. System init captures the resume ID.

Dispatch registers a UUID before writing. Either `started` or `completed`
proves consumption, including a lone `completed` after a result.
`cancelled`, `discarded`, and `refused` reject pending input. Timeout, failed write, and process exit are uncertain. Cancelling
a dispatch removes its pending registration. Runner supplies persisted client
UUIDs and owns retry and deadline policy. Driver's generated UUID helpers serve
the standalone live-check.

Abort sends a stream-json `interrupt` control request and awaits the matching
control response. Its default deadline is CONTROL_REQUEST_TIMEOUT (five seconds); callers can supply a
different deadline. A successful response preserves the Claude process. A
timeout or broken control channel kills the process group. An explicit control
rejection is returned to the caller. Responses also remain on the event stream.
Abort resolves outstanding dispatch waiters as uncertain and never replays
input. A cancelled partial JSON write terminates the process so a subsequent
frame cannot complete a corrupted message.

The control channel correlates replies independently of message delivery. Model
selection uses `set_model` without restarting Claude; effort uses
`apply_flag_settings` with `settings.effortLevel` (ADR 0006).

The event receiver holds at most EVENT_CHANNEL_CAPACITY (1024) events and applies backpressure without
discarding events. The reader still watches process exit while its channel is
full: it kills the process group then, and pending input settles once buffered output
has drained. Callers must continuously drain events;
Runner drains them and persists each event before processing it.

Fake-child tests use Python 3 and local sleep processes. The live-check uses a
fresh directory, a fixed steering timer, a total deadline, and asserts STEERED
with no WRONG before succeeding. It always stops its Claude process.

Claude runs in bypass-permissions mode with permission prompts set to none.
AskUserQuestion, EnterPlanMode, and ExitPlanMode are disabled. A system-prompt
addition tells the agent to state missing input in text and end its turn, so
the owner can answer later. Resume passes the stored Claude ID explicitly and
does not fall back to a new session on failure.
