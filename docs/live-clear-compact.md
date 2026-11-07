# Live check: /compact and /clear

Run on Claude Code 2.1.308 with `crates/live-check/src/bin/context.rs` through our driver
(stream-json input), in an isolated `CLAUDE_CONFIG_DIR` under `target/` with copied
credentials, removed afterward. Haiku, one short fact, then each command.

| Step | Result |
| --- | --- |
| Tell Claude the secret word BANANA-42 | session ID `first session` |
| `/compact` | same session ID; `system compact_boundary`; result success |
| Ask for the word | `BANANA-42` (the summary kept it) |
| `/clear` | `system init` with a new session ID `second session`; result success |
| Ask for the word | `UNKNOWN` |
| Stop, resume `second session` in a new process, ask again | `UNKNOWN`, same ID |

Each command produced `Queued`, `Started`, one `result` and `Completed`, so a slash
command never leaves a turn open.

Raw event trace: kept in the check's output only (the check's working files were deleted).
