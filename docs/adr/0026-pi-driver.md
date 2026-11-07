# 0026: Pi chats through DeepInfra

## Runtime and model choice

A Pi chat runs one `pi --mode rpc` process group behind the runner's Driver interface.
The tuning catalog offers DeepInfra's DeepSeek model IDs, with agent `pi` and two
thinking levels, medium and high (the default). Every Pi chat reasons; DeepInfra
disables reasoning unless the request asks for it.
The model determines the agent for the life of the chat (ADR 0023). Pi uses the
`deepinfra` provider. The catalog offers only `deepseek-ai/DeepSeek-V4.1-Flash-1017`, DeepSeek's update to
V4.1 Flash with better reasoning, coding and agentic results and shorter output. The
plain `DeepSeek-V4.1-Flash` ID is a separate, older model on DeepInfra, not an alias. The driver writes a credential-free models.json from that catalog
on each spawn, using DeepInfra's OpenAI-compatible API with Pi's `deepseek` thinking
format. Pi then sends each level as DeepSeek's own `reasoning_effort` with
`thinking: {type: enabled}`. Pi's Anthropic-format path sent a thinking budget that
DeepInfra did not enforce, so medium and high behaved alike and one turn reasoned until
it hit the output cap. The compat block also turns off the developer role and uses
`max_tokens`, both of which DeepInfra requires. DeepSeek's max is reachable later by
offering xhigh with a `thinkingLevelMap` entry mapping it to `max`; it is not offered
because a test showed no gain on hard questions for about twice the time.

DeepInfra was chosen over Fireworks on cost and retention. Its default terms keep
prompts in memory only, delete outputs once returned, log only request metadata,
and do not train on inputs, without an enterprise contract. Automatic prompt caching
works on both its endpoints, and Pi reads cache hits from the OpenAI usage fields.
Processing is in the United States; there is no region pinning. Context size, output limit and estimated
per-token prices live in tuning. Pi's reported cost is an estimate using those
prices, not a billing receipt.

The driver owns JSONL framing, request correlation, event translation and process
cleanup. It tolerates unknown fields, unknown events, malformed lines and oversized
lines. Stderr is discarded because agent diagnostics can contain credentials.
Startup, requests and shutdown have bounded deadlines. Exit, failed startup and drop kill
the whole process group. The runner disconnects after uncertain request failures. The runner's delivery deadlines remain authoritative.

Pi's configuration and session files live under `pi/<chat-id>` beside the BobLM
database, with directory mode 0700. They must survive reboots. The owner's
normal Pi configuration cannot override the selected provider or credential.
Automatic extension discovery, skills and prompt-template expansion are disabled.
Only BobLM's explicit peer-tools extension is loaded in normal chats. The asynchronous-owner
instruction is appended to the system prompt. Extension UI dialogs are canceled
immediately; a Pi chat never waits for owner approval or input (ADR 0010).

## Consumption and steering

Pi's successful prompt response means accepted or queued. It does not prove
consumption. Each send records its UUID and exact text in an ordered pending list
before writing. Only a user `message_start` matching the head of that list emits
`command_lifecycle: started` for that UUID. A user event with different content,
a duplicate `message_end`, a turn start, a queue update or an RPC reply proves
nothing. This correlation requires exclusive ownership of Pi input: the loaded
extension registers tools only and never injects user messages. Unknown slash
commands are refused so command handling cannot bypass the consumption rule.

Every ordinary send uses `prompt` with `streamingBehavior: steer`. Pi queues it
at the next tool boundary while busy and starts it immediately while idle, including
a steer arriving after its original turn ended. Plain `steer` is unsuitable because
it queues indefinitely when idle. Pi does not interrupt a running tool for a steer.
No request is replayed after a pipe error, timeout or missing consumption event.
Explicit preflight refusal removes only the refused pending entry and tells the
runner that the message was refused. Any ambiguous outcome stays uncertain.

Pi 0.73.1 emits `agent_end` before its automatic retry and compaction work. The driver
disables Pi's own turn-level retries and automatic compaction so that `agent_end` is a
reliable turn boundary. Private settings set `retry.provider.maxRetries` to
`PI_PROVIDER_MAX_RETRIES` (2). These are the provider SDK's retries inside one request,
before `agent_end`, so they keep turn boundaries reliable while recovering from a
connection error, 408, 409, 429 or 5xx. The tradeoff is an occasional double-billed
call when a response is lost after the provider accepted it. Project `.pi/settings.json`
overrides global settings, so startup rejects project overrides that enable turn
retries or compaction or that change the provider retry count. `/compact`
remains available. `turn_end` is an individual model/tool iteration, not the end of
the owner's turn. Abort sends `abort`; its acknowledgment alone does not manufacture
a result. The aborted assistant and `agent_end` provide the result. After the abort response,
the driver stops the process group: Pi leaves unconsumed steers in memory on abort.
A later start resumes persisted history, with uncertain input held by the runner.

## boblm-driven compaction

Pi's own automatic compaction stays off for the same `agent_end` reason. The runner
owns a boblm-driven alternative because it already holds the queue, the turn state and
the chat's recorded context. After a Pi turn completes at or above
`PI_AUTO_COMPACT_PERCENT` (98) of the context window, the runner queues `/compact`
through the ordinary delivery path and dispatches it before any queued message. It
never sends compaction while a turn is active and never discards or replays queued or
uncertain input. The command is an automated message, so it does not reopen a stopped
chat or lift a hold.

Turn-end compaction is the late, clean path. A Pi turn that fails with a provider
context overflow ends with the result subtype
`context_overflow` instead of the generic execution error. The runner compacts and
then queues one short `boblm: continue` message, also through the ordinary delivery
path. The automatic resume happens at most once per overflow chain: a second overflow
ends the turn without another compaction or resume, and a successful non-automatic
turn resets the allowance. A successful auto compaction clears context through the
existing usage trigger, so the same threshold crossing cannot enqueue a second
compaction.

## History, commands and events

The stored agent conversation identifier is Pi's absolute session-file path.
Startup checks that a requested resume file exists and verifies the path reported
by `get_state`; a missing or mismatched resume never starts a replacement conversation.
`/clear` calls `new_session`, checks `cancelled: false`, then obtains a different
session-file path before announcing the new conversation and command completion.
`/compact` starts only at `compaction_start` and completes at `compaction_end`; a
successful RPC reply alone cannot mark it delivered. Both produce the existing UI
markers. Compaction errors and aborts end the command without inventing success.

Assistant text, thinking and tool calls become Claude-shaped content blocks. The
translator gives streamed assistant messages stable IDs, emits final content once,
and preserves input, output, cache-read and cache-write token counts plus Pi's
reported totals and cost in the assistant event. Tool results retain their call IDs
and error status. Provider-side spending limits belong to the owner in the DeepInfra
dashboard; BobLM does not manage spend for home-directory keys.

## Peer tools

Pi has no built-in MCP client. A small embedded JavaScript extension uses Node's
HTTP fetch inside the existing Pi process to initialize BobLM's HTTP MCP endpoint,
list its tools and register their schemas with Pi. Calls go to the same endpoint
as Claude and Codex, with the same session capability in `BOBLM_MCP_TOKEN`.
The endpoint and token travel in the child environment. Neither is embedded in the
extension file. The extension registers its readiness command only after tool
discovery succeeds; the driver verifies it before allowing a prompt. Failure to
connect fails chat startup instead of silently running without peer tools.
Requests have bounded response sizes and deadlines and do not retry tool calls.

## Credentials

`PI_SOPS_FILE` is `~/.config/sops/user-secrets.yaml`; `PI_SOPS_KEY` is the single
object-key path `["DEEPINFRA_API_KEY"]`. Both live in crates/tuning (ADR 0018).
The server resolves the home path and symlink, confirms that the destination is
SOPS-encrypted, and invokes `sops decrypt --extract` for exactly that path on every
Pi spawn. The captured value goes directly into that child's `DEEPINFRA_API_KEY`
environment variable. It is never a command argument, file, event, log or cached
server value. The driver receives no knowledge of the SOPS layout.

The decryption helper follows the existing SOPS runner's bounded process convention,
but discards decryption stderr rather than trusting it to be safe to show. It bounds
stdout, rejects empty, oversized, non-UTF-8 and multiline values, and reports the
file, key path, process status or operating-system cause, and the next action without
showing decrypted output. The reader retains credential copies for the child lifetime
only, to redact accidental echoes before translating stdout into events. Failures refuse startup; there is no ambient-key fallback.
The one-key rule applies to BobLM's credential extraction and injection. Like the
other drivers, Pi runs trusted coding tools as the server's OS user with full
permissions (milestone 1). Its private configuration directory is not a filesystem
sandbox: tool commands can access that user's files, including SOPS identities.
Redaction covers accidental echoes of the injected key and capability, not arbitrary
secrets a tool might read. OS isolation would require a separate agent execution policy.
Rotation applies at the next spawn. The owner can update the entry through
`request_sops_secret` (ADR 0025).

`sops exec-env` decrypts and exports every top-level key from this file. That would
give Pi unrelated credentials, so it is unsuitable even though the keys already
have environment-variable names. Pi also supports `!command` and environment names
in its auth/model config; auth-file commands cache their result for the process
lifetime, while models.json commands resolve per request. Those paths require
configuration outside the driver and make BobLM's bounded, value-free failure
reporting harder. Extracting one entry at the server's spawn boundary keeps the
credential policy in the application that owns it.

## Verification boundary

`docs/live-pi-protocol.md` separates observations from installed Pi with a local
simulated provider from paid Fireworks evidence. Driver behavior is tested through
PiSession with a Python fake child, including consumption, late steering, refusal,
interrupt, resume, compaction, clear, usage, malformed output and process cleanup.
Runner tests with the fake child cover the threshold crossing, no compaction below it,
one compaction before queued input, overflow then compact then one resume, and no
double compaction.
Server tests exercise one-key SOPS extraction with disposable keys under target/.
Browser tests cover the Pi label, model grouping and effort choices. Mutation checks
verify that the delivery and control assertions detect broken behavior.
