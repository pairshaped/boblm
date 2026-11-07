# 0006: Claude model, effort, and stop controls

The tuning crate owns the model catalog (`MODELS`, ADR 0023). Each entry names a
model, the agent that runs it (Claude, Codex or Pi) and the efforts it accepts; the
wire carries model and effort as plain strings and the server checks them against
the catalog with an error that lists the valid choices. The Claude entries are
opus, sonnet, and haiku. Model and effort are stored per session. The verified Claude Code
2.1.306 list_models reply on this host reports low, medium, high, xhigh, and max
for each alias. The catalog exposes those choices in listSessions, so browser
code does not maintain a separate copy. Provider aliases can change; updating
this catalog requires rechecking the running CLI's reported capabilities.

Model changes use the set_model control request with a model string. The driver
also accepts null to reset to the CLI default. Effort changes use
apply_flag_settings with settings.effortLevel set to the selected string, or
null to clear the override. Each control has a correlated response and a bounded
CONTROL_REQUEST_TIMEOUT (five seconds). A timeout terminates the process instead of continuing
with unknown configuration. A model or effort control failure prevents queued
input from dispatching under a configuration that did not apply.

set_max_thinking_tokens changes a thinking-token budget. It is not the effort
selector. The live probe showed that apply_flag_settings changed
get_settings.effective.effortLevel from low to high. The corresponding
[effort documentation](https://platform.claude.com/docs/en/build-with-claude/effort)
describes effort separately from thinking budgets. No token-budget mapping is
used by BobLM.

At an idle turn boundary Runner applies the stored choice before writing the
next input. It skips redundant controls when the choice already applies to the
current process. A newly resumed process reapplies the persisted selection.
Selection during an active turn is persisted immediately. New messages keep
steering under the current configuration. The choice applies only when the
next idle-started turn begins, after outstanding acknowledgments settle.
SessionStatus.selectionPending and durable state_summary events make the
stored but unapplied choice visible. The session row records the choice most
recently applied to a process; selectionPending is true while it differs from
the stored choice, so disconnect, archive and restart cannot hide it. Once the
driver applies the choice, the next summary clears selectionPending. A newly
started process always receives the stored choice before its first input.

Stop writes interrupt and waits for the turn result or process exit, not the
control acknowledgment. Its STOP_RESULT_DEADLINE (30 seconds) falls back to process
group termination. New input can be accepted durably while stop is pending but
cannot dispatch until completion. Unacknowledged input becomes uncertain;
queued input is held. Archive adds process shutdown and retains history and
selection. Reopen alone performs no execution.

## Live verification

The explicit controls live-check ran against Claude Code 2.1.306 in an isolated
configuration directory under target. It changed a completed Sonnet turn to
Haiku, verified effortLevel=high through get_settings, and retained the same
Claude session ID. It interrupted Bash after its READY marker, observed the
control reply before the final result, and received that result in 0.01 seconds
without the WRONG marker. A following Haiku turn completed in the same session.
The source executable is crates/live-check/src/bin/controls.rs; the local run
output is target/model-stop-live.log.
