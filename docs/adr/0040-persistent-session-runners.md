# Persistent session runners

Each chat's runner owns its serial delivery actor, provider process, communication
pipes and, for chats started before the role cutover, native child agents.
Native subagents are disabled for new provider launches (ADR 0043). The web server owns HTTP, browser streams, scheduled
prompts, questions and push delivery. Losing a web connection leaves the runner
working. Server-backed tools can be unavailable during the restart; durable owner
input and pending answers remain in SQLite until they can be handed off.

The production DriverFactory attaches to a runner through a private Unix socket.
Tests can keep using local actors with injected fake drivers. Both adapters expose
the same SessionHandle interface, including durable message UUID receipts and
consumption acknowledgments. Web-server shutdown aborts proxy tasks without sending
the actor's shutdown action. Event pages continue to read the runner's SQLite log,
so reconnecting browsers catch up by cursor.

The runner control protocol has an explicit version, chat identity, bounded frames
and bounded client lifetimes. Socket directories are private and sockets are mode
0600. A read-only handshake verifies the chat and protocol before attachment.
Mutations are never automatically retried after a missing reply. A read-only status
refresh can reconnect without replaying commands. Missing runner status marks the
job inventory unavailable until a fresh status arrives. An unreachable or
incompatible runner leaves that chat unavailable while other chats can load.
Its proxy can reconnect when the socket recovers, without replaying failed input.
The proxy also restarts a runner that has gone away: when a status poll finds no
socket, it asks the launcher to start the chat's unit again, at most once per
`RUNNER_RELAUNCH_INTERVAL`. Starting is idempotent because the chat's file lock
admits one runner. A protocol mismatch means a live runner and is never replaced.

User systemd units supervise runners independently of the web unit. Unit names and
sockets include hashes of the canonical database path and chat ID, so production
and dev instances cannot share runners. A new runner must acquire the chat's file
lock before removing a stale socket or recovering messages. Concurrent launchers
therefore cannot create two delivery actors. `KillMode=control-group` removes the
provider descendants after a runner failure. Restart-on-failure creates a new
actor that holds queued input and marks unacknowledged dispatches uncertain. The
owner's Stop decision is stored separately and survives this recovery.

A graceful runner shutdown records a quiet-stop marker only when there is no
active turn, compaction, pending acknowledgment, queued or uncertain input, or
background job. A running provider needs an available, empty job inventory to
prove quiet. Durable input is checked again in the marker's write transaction.
The marker is written after shutdown cleanup and stored in an add-only column
on `runner_state`, so live runners can continue using the table.
A replacement actor consumes and clears the marker before doing any recovery.
It releases the queue only for a quiet marker with no dispatching or uncertain
input at startup, and only when the owner has not stopped the chat. Later owner
and peer messages then dispatch normally. A crash without that marker retains
the hold. An uncertain message is never replayed automatically.

`boblm restart-runner --session ID [--wait SECONDS] [--config PATH]` waits for
one chat to become quiet, then stops only its database-scoped runner unit. It
uses the same unit identity and socket handshake as runner supervision. The
newest activity event must be a turn result at least `RUNNER_QUIET_PERIOD` old
(20 seconds). Activity includes assistant, user, stream and result events, turn
starts (`init`, `turn_started`, `task_started`, status `requesting`), compaction
starts and command-lifecycle `started`. Operational events such as usage polls,
job inventories and state summaries do not reset the activity clock. Live status
and durable input must show no active turn, stopping, queued, dispatching or
uncertain input, and no jobs. Unknown job inventory cannot prove quiet while a
provider is running. The default wait is
`RUNNER_RESTART_WAIT` (30 minutes). Timeout leaves the unit running and reports
why the restart was refused. The web proxy starts the replacement on its next
reconnect. Work arriving between the quiet check and shutdown is checked by the
actor at shutdown and keeps recovery held if it made the chat busy.

Codex completion observation runs with the session runner, so a child completing
while the web server is down can still wake its parent. Background-job observation
and delivery already live with the provider driver and actor. Archive stops the
provider while retaining the runner; deleting a chat lets its actor and unit exit.
Stale lock files are harmless: ownership is the live file lock, never a saved PID.
Cleanup uses known systemd units rather than process-name matching or global kills.

Existing runner processes keep their loaded code when a new executable is
installed. The control protocol must remain compatible with those runners. An
incompatible driver or state-machine upgrade requires an explicit per-runner
restart after its work is safe to interrupt. Database migrations must also remain
compatible with live old runners. Ordinary web and MCP-handler changes can deploy
without replacing the runners; provider tool-schema caching can still require a
later tool refresh or a per-chat restart to expose new tool definitions.

The authenticated runtime-capability reply identifies the web process and its
runner ownership. `bin/restart` bypasses the legacy idle guard only when that PID
matches the supervised web unit. A legacy process cannot make this claim, so the
first rollout still uses its existing guard or an explicit owner override. Dev
restart replaces only the web process; dev stop explicitly stops that database's
runner units as well. `boblm stop-runners --config PATH` is the explicit instance
shutdown operation and interrupts its agents and jobs.

Random-port instances (`port: 0`) retain the in-process adapter: reconnecting
surviving agents would otherwise leave them pointing at the previous MCP port.
Their runtime capabilities report legacy ownership. Production and normal dev
instances use fixed ports and the persistent adapter.
