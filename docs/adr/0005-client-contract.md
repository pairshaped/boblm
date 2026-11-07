# 0005: HTTP commands and durable SSE events

Rust serde types in protocol::client own the browser contract. Schemars derives
JSON schemas from those types. The protocol-generate example emits TypeScript
types, schema-backed decoders, and Rust JSON fixtures. bin/check runs generation
with --check, so changing a Rust field without updating generated output fails.
Closed application types reject missing required fields, wrong types, unknown
enum values and extra fields. Claude event payloads remain open JSON. Bodies
and event JSON are limited to MAX_BODY_BYTES (1 MiB); sequences must fit JavaScript safe integers.

## Commands

Commands use HTTP POST with plain JSON bodies. The command endpoint determines
the body and reply type; there is no operation tag, wire version, request ID,
or payload wrapper. The command names are listSessions, listDirectories, addProject, removeProject, reorderProjects, reorderChats, createSession,
sendMessage, stopSession, archiveSession, reopenSession, resumeSession,
retryMessage, setModel, runtimeCapabilities, chatHistory, subagentHistory,
renameProject, renameSession, deleteSession, submitSecret, pushConfig, subscribePush,
unsubscribePush, chatPresence, listQuestions, answerQuestion and discussQuestionLater. AddProject and RemoveProject take a project ID and
return the project or Empty; CreateSession takes a session ID, a project ID,
a model and an optional effort, and never a folder (ADR 0023). There is no operation-name regex constraint.

ListDirectories takes an optional absolute path and returns a DirectoryListing
of folders inside the server user's home directory (ADR 0028).

ReorderProjects takes projectIds in their requested order. ReorderChats takes a
projectId and sessionIds in their requested order. Both return Empty, preserve the
positions of unnamed items, and refuse duplicates or unknown IDs; chats must belong
to the named project (ADR 0027). The browser refetches the list after every reorder
reply and never automatically repeats a reorder.

ListSessions uses Empty and returns ListSessionsResponse, including a global
list-event cursor captured with the session and project rows in one SQLite read
transaction. The browser loads this snapshot before opening the list stream and
resumes after its cursor. Fresh snapshots replace the list stream, while ordinary
live events advance its cursor without replacing it. Returning to a hidden page
loads a fresh list before resuming the stream. Historical working and idle states
therefore do not replay through the sidebar on reload or tab return.

ListSessions uses Empty and returns ListSessionsResponse. CreateSession uses
CreateSession and returns SessionInfo. SendMessage uses SendMessage and returns
SendMessageResponse. RetryMessage and SetModel use their corresponding body
types and return CommandResponse. Stop, archive, reopen and resume use
SessionRef and return CommandResponse. Non-success HTTP responses carry
ErrorInfo. The Server task owns command routes, authentication and handlers.

SendMessage carries the client-generated message UUID. The server persists
acceptance before replying. An identical UUID retry refers to the same message;
conflicting content fails. A network failure before a valid reply is uncertain.
The browser never automatically replays commands. A deliberate resend must keep
the original UUID. This HTTP uncertainty is separate from Runner's durable
uncertain delivery state: losing a reply does not prove any agent missed input. The rule applies to Claude,
Codex and Pi.

Uploads are the one non-JSON request: ADR 0017 specifies the upload and download
routes, and `attachments` on SendMessage.

## Session events

GET /sessions/{session_id}/events returns text/event-stream. Last-Event-ID is
the last received per-session sequence, defaulting to zero. Invalid, negative,
non-integer, duplicate or unsafe IDs fail with HTTP 400 before streaming.
Each event has event: sessionEvent, id equal to its sequence, and data containing
the JSON StoredEvent. The browser checks that the payload session and sequence
match the subscribed session and SSE ID before accepting the cursor.

Runner's cursor_page is the only typed durable page reader. It reads strictly
after the cursor and never advances server state. The axum adapter requests
one event per page and yields it directly under HTTP body backpressure. It has
no producer task or subscriber channel. Idle reads poll every SSE_POLL_INTERVAL (50 ms) and axum
sends keep-alive comments every SSE_KEEP_ALIVE (15 seconds). A slow reader resumes from SQLite.
Dropping the body drops the stream and its pending read. Oversized events fail
the stream without skipping their sequence.

The router is a transport component to mount behind Server's bearer-token
middleware. It does not open a listener or assemble the application. Session
handle ownership and command routes remain the Server task's responsibility.

The browser streams through Hypertea's live subscription (`@pairshaped/hypertea/live`) with
its fetch-based event-stream transport, because the stream needs the bearer token and
`Last-Event-ID`. `web/src/stream.ts` is BobLM's thin wrapper: the bearer header, the header
cursor, a fixed one-second retry, the record size cap, and BobLM's wording for failures (the
token, the Tailscale link), chosen by switching on Hypertea's structured failure kinds. Each stream's key names its resource and excludes the changing
cursor. The app's decoder refuses an event that is out of order or for another chat, and a
refused event never advances the cursor; the stream reconnects from the last accepted event.
Disposal aborts reads and retry timers. A stream the server closes is reported as an
interruption and resumed from the last event. The streams sit in fixed subscription slots
(the list, open chat, usage-reset timer and subagent stream, with empty slots
for absent resources), because Hypertea matches subscriptions by
position.

## Session-list stream

GET /sessions/events resumes using a separate durable global sequence from
session_list_events. This small table contains created, renamed, archived, deleted,
state_summary and subagents_changed events. Per-session streams retain their own sequence. Created
and archive changes append to the list in the same transaction as the session
change. Runner publishes changed state summaries to both streams atomically;
ordinary agent output never goes into the list table. Startup publishes the
recovered state without starting Claude. The list cursor page is also owned by
Runner and the adapter reads one event at a time under backpressure.

List SSE frames use event: sessionListEvent and JSON SessionListEvent data.
Their ID is the global list sequence. A reconnect test spans several sessions,
archive changes and reopening the database, with no skipped or repeated IDs.
Renamed is a closed payload shape for list consumers; renameSession updates the title.

## Verification

Rust fixtures cover every command body, response shape, delivery state and
application event variant. Browser negative tests cover invalid fields, enums,
size limits and unsafe sequences. An axum body test pauses consumption while
SQLite grows, disconnects, then resumes using Last-Event-ID and receives all
61 events in order. Browser tests cover chunk boundaries, reconnects, aborted
subscriptions, malformed event retries and uncertain commands retaining UUIDs.
