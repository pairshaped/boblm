# Live check: slash commands and results

Run on Claude Code 2.1.308 with `crates/live-check/src/bin/slash.rs`, in an
isolated `CLAUDE_CONFIG_DIR` under `target/live` with copied credentials, removed
afterward. Haiku, one short turn, then `/cost`, then `/compact`.

Each of the three inputs produced `lifecycle Queued`, `Started`, exactly one
`result` (subtype `success`, `is_error` false), then `lifecycle Completed`. The
slash commands also emitted `system` events (`init`, `status`, and for `/compact`
`compact_boundary`) before the result. No slash command completed without a
result, so the runner's active flag is always cleared by a result.

Raw output: `target/live/slash.log` (local, not committed).
