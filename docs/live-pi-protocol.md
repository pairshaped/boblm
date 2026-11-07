# Live check: Pi RPC

## Installed protocol, local provider

Probed Pi 0.73.1 (`@mariozechner/pi-coding-agent`) on 2026-10-23 through
`pi --mode rpc`. Configuration and sessions were isolated under target/pi-probe.
A local HTTP server supplied deterministic Anthropic-format stream responses;
no credential or paid provider call was used. Pi's bundled Fireworks models use
`anthropic-messages`, not OpenAI chat completions. These observations establish
Pi's protocol behavior, not Fireworks account availability or actual billing.

| Check | Observed evidence |
| --- | --- |
| Prompt consumption | `response` with `command: prompt, success: true`, then `agent_start`, `turn_start`, user `message_start`, user `message_end`, assistant `message_start`. User events contain exact text and a timestamp, but no request/client UUID. |
| Active steering | A second `prompt` with `streamingBehavior: steer` returned success and `queue_update` while the first answer streamed. After the first `turn_end`, the second user `message_start` appeared, followed by its answer and one enclosing `agent_end`. |
| Late steering | Plain `steer` while idle returned success and left `pendingMessageCount: 1`, without starting a turn. `prompt` with `streamingBehavior: steer` after `agent_end` started a fresh turn. |
| Tool execution | A simulated provider tool call made real Pi run `sleep 1; printf local-tool`. It emitted `tool_execution_start`, updates and end, a toolResult message, then another model iteration. |
| Interrupt | `abort` during a delayed local response produced an assistant `message_end` with `stopReason: aborted`, then `turn_end`, `agent_end`, and the successful abort response. |
| Resume | A second Pi process launched with `--session <sessionFile>` reported the identical session ID and file, and `get_messages` returned 11 messages including the compacted history. |
| Tokens | A supplied usage sample of 42 input, 3 output and 10 cached tokens became `usage: {input:42,output:3,cacheRead:10,cacheWrite:0,totalTokens:55}`. Five model iterations totaled input 210, output 15, cacheRead 50, total 275 in `get_session_stats`. Pi computed cost 0.0001568 from its catalog. These are simulated tokens, not charges. |
| Models | `get_available_models` returned Fireworks IDs `accounts/fireworks/models/deepseek-v3p1`, `accounts/fireworks/models/deepseek-v3p2`, `accounts/fireworks/models/deepseek-v4-pro`. All have reasoning enabled. |
| Thinking | On each model, setting off/minimal/low/medium/high read back unchanged in `get_state`. xhigh read back high; unsupported max read back off. Neither belongs in BobLM's catalog. |
| Compact | `compact` emitted `compaction_start` with reason manual, then `compaction_end` with a summary, firstKeptEntryId and tokensBefore, before its successful response. |
| Clear | `new_session` returned `data.cancelled: false` and a later `get_state` reported a new file and ID. |

The installed source (`dist/core/agent-session.js`, `_processAgentEvent`) emits
`agent_end` before automatic retry/compaction. Unlike newer upstream documentation,
this installed build does not supply `agent_settled`. Disable both automatic modes
and use manual compaction so the driver can safely expose a result.

The installed README explicitly says there is no built-in MCP. The extension loader
awaits an async extension factory, and `registerTool` accepts tool schemas. The
installed RPC implementation maps select/confirm/input/editor dialogs to
`extension_ui_request`; `{type:"extension_ui_response",id,cancelled:true}` resolves
them without waiting for the owner. Unknown stdout events and non-JSON lines need
tolerance; stderr is diagnostic output, never delivery evidence. The driver discards
stderr to avoid retaining provider credentials in error dumps.

A follow-up local check registered both current V4.1 Flash IDs and read back all
five thinking levels unchanged. A real extension confirm dialog emitted
`extension_ui_request`; sending `cancelled: true` let a subsequent `get_state`
complete. A local HTTP 429 endpoint received exactly one request with
`retry.provider.maxRetries: 0`; Pi ended the turn with the rate-limit error.
This SDK retry policy is separate from `set_auto_retry`. Installed
`agent-session.js:abort()` stops execution but does not call `clearQueue()`, so the
driver kills the process group after abort to discard pending steers.

## Credentials and paid provider evidence

The configured file is `~/.config/sops/user-secrets.yaml`, resolved through its
symlink, and the selected top-level key is `FIREWORKS_API_KEY`. No other key is
extracted. Paid checks run only through BobLM's spawn boundary, with short prompts,
a 128-token output cap supplied by a probe extension and Pi automatic retries disabled.
There were eight model-request attempts including the rejected model and the aborted
request. No probe retried a failed request. Provider SDK retries were discovered and
disabled after these paid checks; no paid response observed a transient error that
would trigger that SDK retry policy. The local 429 check verifies the final policy.

The installed catalog's `deepseek-v3p2` returned HTTP 404, "Model not found,
inaccessible, and/or not deployed". Fireworks' current
[serverless pricing](https://docs.fireworks.ai/serverless/pricing) and model pages for
[V4.1 Flash](https://app.fireworks.ai/models/fireworks/deepseek-v4p1-flash) and
[its 1017 snapshot](https://app.fireworks.ai/models/fireworks/deepseek-v4p1-flash-1017)
list the replacement IDs. Both list $0.30 input, $0.006 cached input and $1.20 output
per million tokens. The driver registers these models explicitly because Pi 0.73.1
predates them. All successful paid checks used the base V4.1 Flash ID with thinking off.

| Attempt | Input | Cached input | Output | Total tokens | Pi estimated USD | Evidence |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| Obsolete v3p2 | 0 | 0 | 0 | 0 | 0 | HTTP 404, no retry |
| Remember | 1318 | 0 | 3 | 1321 | 0.000399 | `Started`, answer `OK cobalt`, success |
| Resume in new process | 1337 | 0 | 3 | 1340 | 0.0004047 | Answer `cobalt`, same session file |
| Tools with incorrect probe flag | 1318 | 0 | 105 | 1423 | 0.0005214 | Model reported no list_projects tool |
| Corrected tools, call | 2166 | 1174 | 31 | 3371 | 0.000694044 | `tool_use: list_projects`, successful HTTP MCP result |
| Corrected tools, answer | 1521 | 1920 | 51 | 3492 | 0.00052902 | Named the isolated project `work` |
| Steering, tool iteration | 1684 | 0 | 39 | 1723 | 0.000552 | Bash `sleep 2`, successful tool result |
| Steering, aborted iteration | 0 | 0 | 0 | 0 | 0 | Second UUID `Started`, abort, result interrupted |
| **Reported total** | **9344** | **3094** | **232** | **12670** | **0.003100164** | No cache writes reported |

The tools probe initially used `--no-tools`, which disables extension tools too.
`--no-builtin-tools` kept the registered MCP tools available and the corrected check
passed. Production uses the normal built-in tools plus the peer-tools extension.
The steering check sent its second prompt while the shell sleep was running and
aborted immediately after the second user message was consumed. The aborted request
reported no tokens; provider billing for an aborted request cannot be inferred from
that zero. Pi's reported costs use the configured price table, not Fireworks invoices.

## Test sensitivity and limits

Fake-child tests cover delivery, duplicate text, late steering, explicit refusal,
provider errors, interrupt, resume, clear, compact, usage, dialogs, malformed output
and process groups. Seventeen mutations each produced assertion failures. The initial nine changed these behaviors:
accept unmatched user text; acknowledge on write; send followUp instead of steer;
zero input usage; accept a mismatched resume; do not cancel a dialog; end on turn_end;
accept an unchanged clear path; extract the wrong SOPS key. Sources were restored
after each mutation. The other eight removed provider-failure reporting, abort process
cleanup, peer-tool readiness, project-policy validation, failed-tool propagation,
the Pi database constraint, readable provider causes, and the Pi browser label.
Disposable SOPS tests also check missing, empty, multiline and
undecryptable entries without returning their values.

Paid checks did not exercise the 1017 snapshot, non-off reasoning, long contexts or
real-provider compaction. Compaction, clear and late steering were established with
real Pi against the local provider and tested through the fake child. No assertion
about a safe 90% context threshold follows from these short checks.

Every Pi process group was stopped and reaped, and `pgrep` found no Pi CLI processes
between runs and after final cleanup. Isolated sessions, probe databases, scripts
and logs under target/ were deleted after verification. The retained example is an explicit live-check entry
point; it never runs as part of the automated test suite.

## Repository verification

On 2026-10-23, `bin/check` exited 0 and ended with `check passed`. It passed
formatting, browser asset build, workspace checks, both generated-file checks,
strict clippy, workspace tests, browser typechecking and all 70 browser tests.
`cargo check -j4 --workspace` and focused `cargo test -j4 --workspace pi_` also passed.
The final source includes the provider-retry and abort-queue fixes; those final
fixes were checked with the local real-Pi probe and fake-child tests, without another
paid call. No production service was restarted and no production database was read.

To reproduce an authorized paid check, first create `target/pi-live`, then run
`cargo run -j4 -p boblm-server --example pi-probe -- remember` once. The other explicit
modes are `resume`, `tools`, `steer` (steer then interrupt), and `interrupt`.
Check for surviving Pi CLI processes and remove `target/pi-live` after the run.
The example requires the configured SOPS file and key; it never accepts a plaintext
credential argument or prints decrypted output.

## Independent review

The independent Luna review found no confirmed driver lifecycle defect. Its credential
finding was reclassified after checking the project's full-permission agent policy;
ADR 0026 states the same-user filesystem boundary explicitly. The confirmed test gap
is covered by a fake that accepts two identical prompts before acknowledging either,
plus refusal of the second duplicate while the first remains pending. Both assertions
failed under deliberate FIFO and refusal-removal mutations. The reviewer report and
its final disposition were read in full, and the reviewer chat was archived.
`bin/check` passed again after these changes.
