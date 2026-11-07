# Context: why BobLM is built this way

Background for anyone steering the project. `docs/milestone-1.md` and the ADRs
say what to build; this says why, and what was deliberately left out.

## What it replaces

BobLM replaces a personal fork of HAPI: a web UI plus hub plus per-session wrapper
processes that supervise Claude Code and Codex. The fork keeps the behavior
the owner wants and drags a lot of cruft. HAPI is behavior reference only. Do not copy its structure.

## Owner preferences that shape decisions

- One user, one machine, one theme (always dark), one font size. No settings
  screens. Behavior is fixed, not configured.
- Everything async. An agent never waits on the owner. No permission prompts
  (YOLO), no blocking question or plan-approval tools. Anything that needs the
  owner becomes a pending item delivered later as a message.
- Phone and browser at once, same account. The server is the only writer.
- Agents: Claude and Codex are subscriptions. DeepSeek (through Pi) is paid
  per token. Other agents are out.
- No push notifications. Reached through Tailscale, installed as a Dock app
  on the Mac and used from a phone.
- Fast, small, boring. Footprint matters because the machine is shared with
  very large Rust builds.

## Stack decisions and why

- Rust server, Hypertea UI. Hypertea is the owner's own TypeScript Elm-style
  runtime (`pairshaped/hypertea`) and already runs a much
  larger admin site. Rust was chosen for footprint, compile-time checking and
  because agents work well in it. The owner dislikes slow Rust builds; a
  small-crate workspace builds in well under a second incrementally (measured).
- Rejected: Effect on TypeScript (faster start and reusable code, but agents
  write it less reliably and a Bun executable is 142 MB per binary); Gleam on
  the BEAM (good supervision, but its SQLite binding is a NIF and the owner
  does not want NIFs); Elm and Lustre for the UI (the composer caret and focus
  problems that this app has hit repeatedly come from model-controlled text
  fields); Postgres (more moving parts for one user).
- SQLite through Marmot (pinned git dependency). No NIFs.
- One process hosts the hub and every session as Tokio tasks. HAPI used one
  Bun wrapper process per session (about 110 to 140 MB each) plus one MCP
  bridge process per agent (60 to 90 MB each, and they accumulate). The
  agents themselves cost more than any of this and cannot be shrunk.
- HTTP commands plus SSE events, no WebSocket. Sends are rare; all
  high-volume traffic is server to browser. Hypertea's socket transport is
  request/reply only and cannot receive pushed frames. The SSE reader here is an app-level Hypertea
  subscription so it can later move into Hypertea unchanged (bean in
  Milestone 2). Hypertea changes happen upstream, not from this repo.
- TLS comes from `tailscale serve` in front of a localhost listener, like
  `hapi-https.service` on this machine. Caddy is not involved.

## Claude Code facts that are easy to get wrong

- A steer is a user message with `priority: "now"`. It redirects at the next
  tool boundary; it does not cancel a tool call that is already running, and
  inside a foreground subagent it kills the subagent. Background subagents
  keep running.
- `set_model` and `interrupt` are control requests over stdin; no restart is
  needed. Stop and archive use `interrupt`.
- A lone `completed` lifecycle event proves consumption just as `started`
  does. A write alone never proves delivery.
- A steer cuts the turn: expect an empty result, the original prompt
  `cancelled`, then the steer `started`. Not a failure.
- Claude connects to an HTTP MCP server directly (`--mcp-config` with
  `type: http`); no stdio bridge is needed for it.

## Codex (Milestone 2)

Protocol facts are recorded in bean `boblm-op3v`. Generate Rust types from
`codex app-server generate-json-schema --out <dir>` instead of hand-writing
them. None of it has been run against a live Codex yet.

## Known gaps after Milestone 1

- No systemd unit or `tailscale serve` service for BobLM yet.
- Screenshot review and the manual Safari check (`docs/safari-checklist.md`)
  are the owner's to do.
- Milestone 2 beans are drafts. Agent peer tools come first and are
  owner-confirmed as needed.

## How the owner works

Numbered points and questions, continuing the numbering across messages.
Brief commit messages, no em dashes, run the formatter before committing,
delete completed beans with `beans delete <id> --force` rather than archiving.
Commit only when asked.
