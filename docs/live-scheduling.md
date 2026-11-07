# Live check: an agent schedules a prompt for itself

Run on Claude Code 2.1.308 with the real `boblm serve` binary (scheduler running), an isolated
`CLAUDE_CONFIG_DIR` under `target/` with copied credentials, removed afterward, Haiku at low
effort.

1. Asked the chat's Claude to use its `schedule_prompt` tool for a one-shot prompt one minute
   ahead, with a stated schedule ID and the prompt "Reply with exactly SCHEDULED-OK".
2. Claude found the tool (`mcp__boblm__schedule_prompt`), called it with `inMinutes: 1`, and got
   the stored schedule back (`state: active`, `nextRunAt` 64 seconds after the request).
3. When the time came the server sent the prompt to the same chat as an ordinary message headed
   "Scheduled prompt:". Claude answered `SCHEDULED-OK`.

Not covered live: repeating schedules, the busy-chat skip, archive and reopen, downtime. Those are
covered by tests with an explicit clock (`crates/server/tests/schedules.rs`).
