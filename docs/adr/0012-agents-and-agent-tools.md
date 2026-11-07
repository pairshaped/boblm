# 0012: Additional agents and agent tools

Claude Code, Codex and Pi (for DeepSeek) are active drivers. Each agent
has its own driver crate behind one narrow interface that the runner uses: start
or resume, send, steer, interrupt, change model, and report lifecycle events.
The runner's delivery rules (ADR 0004) apply to every agent. Agent-specific
protocol detail stays in the driver.

Delivery proof is per agent. Claude proves consumption with `command_lifecycle`
`started` or `completed`. Codex's app-server steers with `turn/steer` carrying
the expected turn id and a client message id, interrupts with `turn/interrupt`,
and takes model and effort on `turn/start`. Codex has consumed a message when it
announces a `userMessage` item whose `clientId` is our client message id; the
driver translates that to the same `started` lifecycle (docs/live-codex-protocol.md).
Codex's `/compact` starts when Codex announces the compaction item, not when it accepts
the request; `/clear` is a new thread, so its outcome is the thread being opened. A steer that
finds no active turn falls back to starting a turn. The driver
speaks only the part of Codex's schema it uses; `crates/codex-driver/schema/surface.json`
records that part, the fake app-server in the driver tests refuses requests that
disagree with it, and `bin/refresh-codex-schema --check` reports drift after a
Codex upgrade.

DeepSeek is accessed through Pi and billed per token. Its driver reports token
counts, resumes sessions and steers at tool boundaries (ADR 0026).

Agents get the same tools to inspect and message each other: list, inspect and
ping other sessions, spawn and archive sessions, and change the current
session's title. They are served from one in-process MCP endpoint over HTTP
inside the server, on the loopback address. There are no per-agent bridge
processes. Claude connects to it directly through its MCP configuration. Codex connects
to the same endpoint over HTTP with a bearer token passed in the environment
(verified live: a Codex chat called `list_projects`). Pinging another session is delivered as a steer.
Tool calls that need the owner follow ADR 0010.

Subscription limits (Claude, Codex) and per-token cost (DeepSeek) are different
metrics. Usage meters and `usage_report` report each in its own terms (ADRs 0037 and 0042).
