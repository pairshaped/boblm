# 0015: Agent peer tools over in-process HTTP

Peer tools use the official rmcp Streamable HTTP implementation in the same
Axum process as the browser API. Each Claude launch receives a strict HTTP MCP
configuration, in a mode 0600 file under a private runtime directory (ADR 0016)
that is removed when the session ends, pointing to /mcp/{sessionId} on the bound
loopback port. The capability never appears on a command line. There is
no bridge executable or per-agent MCP listener. The agent driver owns Claude's
CLI arguments; the server owns the tool handlers and session registry.

The connection binds caller identity. Its bearer capability is HMAC-SHA256 of
a domain separator and session ID, keyed by the owner's configured token.
Capabilities are stable across server restarts and valid for only one caller;
rotating the owner token invalidates them. Agents receive that capability,
not the owner token. Tool arguments cannot override the caller. The MCP
endpoint uses stateless JSON responses; the browser's durable SSE streams
remain unchanged. The SDK enforces the MAX_BODY_BYTES (1 MiB) request limit and loopback hosts.

Discovery lists other unarchived sessions without starting them. Inspection
accepts a full session ID or an unambiguous prefix and returns current state
and bounded recent persisted message events, including tool activity. Exact
IDs take precedence over prefix matches. Archived history stays inspectable.
Title changes affect only the caller, run through its serialized Runner queue,
and append per-session and global-list rename events atomically. The browser
shows the stored title in the chat header. Agents are instructed to call
change_title with a 2-3 word task title and onlyIfUnnamed=true once the task is
clear. Runner checks that condition in the same serialized queue as explicit
renames, so automatic naming preserves an existing title. Renames the owner asks
for omit the condition. The owner's instructions and active skills override the
default title format and renaming policy, so a workflow convention such as a Bean
suffix, or renaming when the scope changes, wins over the generic rule.

Ping uses the existing Runner send path, including priority now on active turns.
It requires a client message UUID: an identical explicit retry returns the same
accepted input, while conflicting reuse fails. The stored message identifies
its source session. Acceptance never claims consumption. Uncertain mutations
are never replayed automatically. Archived peers reject new pings until reopened.

Every peer-delivered message ends with a short, stable reply trailer. A spawn_peer
first message and a ping_peer message use the peer trailer, which names
`reply_to_sender`; spawn_subagent uses the subagent trailer, which says the final
reply reaches the parent automatically and that `message_parent` is only for
mid-task questions. The trailer text is deterministic, so retry identity remains
the original arguments rather than the decorated content.

`reply_to_sender` sends to the session that sent the caller its most recent peer
message, including a spawn first message or a ping. It refuses when the caller has
never received one. `message_parent` sends to the caller's stored parent and
refuses for a root chat. Both reuse the Runner send path, record the caller as the
sender, and keep UUID idempotency. `ping_peer` remains for explicit addressing.
The appended system prompt also explains the message prefixes, these tools, `ask`
for owner decisions, and `inspect_peer`. A parent-side report trailer is used by the subagent report outbox
(ADR 0036).
Scheduling tools (`schedule_prompt`, `list_schedules`, `cancel_schedule`, ADR 0022) and
`request_sops_secret` (ADR 0025) are likewise bound to the caller and act only on its own
chat. Archive uses the existing stop-and-archive operation, including its interrupt
and bounded shutdown. Self-archive can close the caller before a reply, so tool
instructions tell agents to inspect from another session before retrying.

Explicit-model spawn requires model and effort, and the model decides the agent
(ADR 0023). Role-routed spawns resolve model and effort through ADR 0043. Spawn also requires a title of 1 to 200
characters, validated like a rename, and stores it on the child in the same
transaction. The title is part of the receipt arguments, so an identical retry
matches while a different title with the same requestId conflicts. An omitted or
null projectId puts the child in the caller's project; `list_projects` shows the
others.
The required first prompt is accepted for immediate dispatch, without waiting
for Claude to consume it. A plain peer is an independent root. Hidden subagents record parent and root
links and report results back through the durable outbox (ADR 0036).

Every spawn requires a fresh requestId UUID, also used for the child session
and first message IDs. Creation, explicit settings, the first queued input,
durable events and an idempotency receipt commit in one transaction. The
receipt records the bound caller and original arguments only for retry checks.
Conflicting reuse fails. An identical retry returns the existing child and
never resets its later settings or releases held or uncertain input, including
after restart. dispatchFailed reports when input is durable but startup failed;
the first prompt stays queued and held for explicit recovery.

Once submitted, spawn work outlives its HTTP caller. The server tracks at most
MAX_PEER_SPAWNS_IN_FLIGHT (64) in-flight creations, completes registration even if the reply is lost, and
drains those operations before shutting down sessions. A lost HTTP reply cannot
leave a committed child absent from the in-process registry.

The Codex driver connects to the same HTTP MCP endpoint with its bearer
capability supplied through an environment variable.

Reference: [official Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk).
Fake-driver tests exercise the actual MCP HTTP boundary, caller binding,
discovery, prefix errors, title persistence, UUID idempotency, active steering
and archive with retained history. A fake child validates direct HTTP MCP
configuration. Spawn tests cover explicit model/effort requirements, directory
inheritance and override, concurrent duplicate requests, conflicting receipts,
startup failure retaining queued input, restart retaining uncertain input,
transaction rollback, and HTTP cancellation during startup with shutdown draining.


Peer messages carry a `sender_session_id` in their persisted `message_appended`
event. The authenticated peer connection supplies this identity for pings, and
the spawn receipt supplies it for a child's first message. Retry identity includes
the sender when recorded. Identical retries of legacy messages remain no-ops. The browser uses this field to distinguish agent messages from owner
messages; it does not infer authorship from text prefixes. Older events without
recorded sender metadata retain their original presentation.
