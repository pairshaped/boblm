# Live check: the Codex app-server protocol

Run on codex-cli 0.163.0 with `codex app-server` over stdio, an isolated `CODEX_HOME` under
`target/` with copied credentials (removed afterward), model `gpt-6-luna` at low effort, using a
throwaway probe that printed every message. Messages are newline-delimited JSON-RPC.

| Step | Observed |
| --- | --- |
| `initialize` then the `initialized` notification | result carries `userAgent` and `codexHome` |
| `model/list` | `gpt-6.1-sol` (default), `gpt-6-astra`, `gpt-6-sol`, `gpt-6-luna`, `gpt-5.6-sol`, `gpt-5.6-terra`, `gpt-5.6-luna`; efforts `low`, `medium`, `high`, `xhigh`, and `max` on all but the two luna models |
| `thread/start` with `cwd`, `approvalPolicy: "never"`, `sandbox: "danger-full-access"`, `model` | result `{thread:{id}}`, then `thread/started`, `thread/settings/updated` |
| `turn/start` with `input:[{type:"text",text,text_elements:[]}]`, `clientUserMessageId`, `effort` | result `{turn:{id}}` at once; then `turn/started`, `item/started` and `item/completed` for a `userMessage` whose `clientId` is our `clientUserMessageId`, `item/started` for the `agentMessage`, `item/agentMessage/delta`, `item/completed` with the full text, `thread/status/changed` to idle, `turn/completed` |
| `turn/steer` during a running shell command | result `{turnId}`; a `userMessage` item with the steer's `clientId` follows immediately; the command was **not** cut and ran to completion; the final answer used both messages |
| `turn/steer` after the turn ended | JSON-RPC error `-32600 "no active turn to steer"` |
| `turn/interrupt` during a command | result `{}`, then idle and `turn/completed` at once (status `interrupted`) |
| `thread/resume` in a new process | same thread ID; the earlier fact was remembered |
| `thread/compact/start` | result `{}`; a `contextCompaction` item starts and completes inside its own short turn |

A message is therefore consumed when Codex announces its `userMessage` item with our ID as `clientId`;
a write or an accepted request alone is not delivery (AGENTS.md). Server requests never arrived with
approvals set to `never`.

The full schema is large (105 client requests, 85 server notifications, 318 files, 4.5 MB). The
driver uses about a dozen of them.

## Through BobLM

The real `boblm serve` (isolated config, data and `CODEX_HOME` under `target/`, credentials copied
in and removed afterward) with model `gpt-6-luna` at low effort:

1. `addProject` then `createSession` with that project, model and effort created a Codex chat.
2. `sendMessage` moved the message through `queued`, `dispatching` and `started`; `started` came from
   Codex's `userMessage` item carrying our message ID.
3. Asked to use its `list_projects` tool, Codex called `boblm.list_projects` over the HTTP MCP
   endpoint, got the project list back, and answered from it. Peer tools therefore work over Codex
   without a relay. The tool call and its result appear in the chat as a tool use and tool result.

The Codex request fields the driver sends are recorded by `bin/refresh-codex-schema`.
