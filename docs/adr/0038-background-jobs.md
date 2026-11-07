# Background jobs outlive turns

The drawer orders its tabs Agents, Jobs, Questions. Jobs shows the current running
commands for the selected chat and its subagents. Finished jobs disappear; their
output remains in the conversation. The visible list is capped by
`JOBS_VISIBLE_LIMIT`, with the full running count shown above it. A subagent whose
turn has completed while it still owns jobs appears as waiting for jobs.

Drivers report complete job snapshots to the runner. The runner publishes them
in session status and persists that status for the restart guard. Job visibility
therefore survives message-window trimming, chat switching, and browser reloads.
Driver shutdown clears the snapshot and publishes the cleared status. A web-server
restart reconnects to the persistent runner and retains its jobs. An explicit
runner stop or failure terminates that runner's provider processes and jobs.

Codex inventories loaded threads in the chat's dedicated app-server, then reads
`thread/backgroundTerminals/list` for each thread. Both lists are paginated.
Polling continues between turns, starts no agent processes, and makes no inference
requests. Failed or incomplete scans retain the last known jobs and mark the
inventory unavailable. The installed experimental schema defines this interface;
`bin/refresh-codex-schema` records the request surface used by the fake driver.

Claude supplies replacement `background_tasks_changed` snapshots. Task-start
records map a command's parent task to the Agent tool call used by the drawer.
Agent tasks and ambient watchers are excluded from the command list. Missing or
malformed inventories are unavailable, rather than proof that no jobs exist.
Pi does not expose a background-job inventory through this integration.

For legacy servers, `bin/restart` refuses while reported jobs remain or a reported
inventory is unavailable. A confirmed persistent-runner server can restart its web
process without interrupting these jobs (ADR 0040). Its refusal includes the job and chat counts. The existing wait,
settle, and explicit force behavior still applies. This guard covers
agent-reported jobs; commands deliberately detached from the provider's job
registry require separate supervision. Direct service restarts bypass the guard.

Drivers declare whether they wake themselves when background commands finish.
Claude does: its native `task_notification` starts a turn, so BobLM publishes the
replacement job snapshot without queuing another wake. Codex and Pi declare false.
Pi's integrated bash tool waits for the command's exit and supplies no background
inventory or completion wake. Snapshot persistence and status publication are
independent of this capability.

For a driver that does not wake itself, a complete inventory removing a previously
observed parent-owned command makes the runner queue an ordinary message saying
it is no longer listed as running. That message starts an idle turn or steers a
mid-turn chat. It asks the agent to check the output and exit status: disappearance
alone does not prove success. Child-owned commands use the child's completion
path rather than sending extra parent notifications.

Failed scans retain the last known tasks. Duplicate snapshots produce no extra
messages. The wake is persisted before dispatch and uses normal consumption
acknowledgments. Stop, archive and uncertainty holds remain in force; queued
wakes wait for the owner to resume. Disconnect clears inventory without creating
completion messages. Commands that finish before the inventory observes them
and commands deliberately detached from the provider registry require an explicit
completion callback.

## Keep Claude background work attached

BobLM appends two rules for Claude, Codex and Pi: background commands run the real
work directly, and agents perform their next step or state exactly what they are
waiting for before ending a turn.

Claude launches include inline JSON through `--settings`, registering a
`PreToolUse` command hook matched to `Bash`. The hook invokes the absolute,
shell-quoted path of the running BobLM executable with `hook-background-bash`.
It does not write to the owner's Claude settings. The installed Claude Code
2.1.318 reports `--settings <file-or-json>` in `claude --help`. The
[hook reference](https://code.claude.com/docs/en/hooks) documents stdin JSON,
Bash matching, and exit code 2 with a stderr explanation to block a tool call.

The subcommand runs before the async runtime starts. It blocks only recognized
PreToolUse Bash input with `run_in_background: true` and a shell background
operator or `nohup`, `setsid` or `disown` in command position. Quoted arguments,
escaped ampersands, comments, logical AND and redirects are allowed. Foreground,
malformed and unknown hook input is allowed. This is an accident guard, not a
shell sandbox: commands hidden inside scripts or dynamically constructed shell
programs still require the agent to follow the prompt rule.

Parser tables and executable-level tests cover allow and block decisions.
The fake Claude child checks the launch settings without starting a real model
session.

## Idle follow-up

A root turn started by a runner completion notice or an observable Claude native
`task_notification` can receive one idle nudge after it ends (ADR 0046). The server
requires an available empty job inventory, or no provider process, and drops the
opportunity when other work, input or an owner question remains.
