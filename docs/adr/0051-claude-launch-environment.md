# 0051: Claude launch environment

Every Claude child starts with `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` and
`DISABLE_TELEMETRY=1`, set from `CLAUDE_ENV` in `crates/tuning`. BobLM runs Claude headless,
so telemetry and error reports add nothing. The native install updates itself, which could
replace the CLI between chat launches while BobLM depends on its stream format; the first
flag turns the updater off along with error reporting and telemetry. The second names
telemetry on its own, so it still holds if the first one's scope changes.

`CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY` is not set. The survey is a terminal prompt, and a
headless launch has no terminal to show it in.

These flags also stop Claude Code downloading some of its feature settings. A real turn with
them set gave the same usage, background-job and result events as one without them, and prompt
suggestions (ADR 0044) still arrived. Suggestions depend on one of those downloaded settings, so
if they stop appearing after a CLI update, add `CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION=1` to
`CLAUDE_ENV`; that variable overrides the setting.

CLI updates are now a deliberate step: update the installed `claude`, then start new chats.
Running chats keep the process they already have, and each launch reads the environment
afresh, so a change here reaches a chat the next time its child starts.
