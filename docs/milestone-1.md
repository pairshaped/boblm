# Milestone 1: one Claude session, end to end

Goal: from a browser, create a Claude Code session, chat with it, steer it
mid-turn, and see everything persisted in SQLite. Everything is one process on
one machine. This is a rewrite of the HAPI fork at `/home/user/projects/example`; that repo
is the behavior reference only. Do not copy its structure.

## Decisions already made

- Server: Rust, small crates in one workspace. Tokio for agent processes.
- Storage: SQLite through `rusqlite` (bundled). Marmot generates typed query
  code from colocated `.sql` files and owns migrations. Marmot is pinned to a revision of `pairshaped/marmot-rust`.
- Contracts: `serde` types in Rust are the source of truth. TypeScript types
  and decoders for the browser are generated from them, with JSON fixtures
  produced by Rust and parsed by TypeScript tests so drift fails CI.
- Transport: no WebSocket. Commands are HTTP POSTs that carry the client
  message UUID (idempotent retries). Events are Server-Sent Events: one
  stream per open session and one for the session list, resuming through
  `Last-Event-ID`, which is the per-session event sequence. A network failure
  before a reply is the `uncertain` outcome; never replay it automatically.
- UI: Hypertea (TypeScript, Elm-style). Source is the public `pairshaped/hypertea`
  repository (`@pairshaped/hypertea`), pinned by commit. The SSE reader is an app-level Hypertea
  subscription (see `lockSubscription` in
  an application effects module
  for the pattern). Build it so it can be extracted into Hypertea later and
  reused by the results and scoring islands; do not change Hypertea in this
  milestone.
- One process: the hub and the runner are modules in one binary (`boblm serve`).
  Sessions are Tokio tasks, not separate wrapper processes. Keep the hub and
  runner as separate crates so a later split stays possible.
- No settings screens, no theme options, no terminal, no voice, no push, no
  scratchlist, no localization. Always dark. YOLO only (bypass permissions).
- One machine only. There is no machine entity: no `machine_id` column, no
  machine registry, no per-machine settings, no remote runners, and no
  `machineId` field in the contract. Session working directories are plain
  paths on this host.
- Agents later: Codex, then Pi (for DeepSeek). Milestone 1 is Claude only.
- Later milestones are listed in the beans under the Milestone 2 bean.
- Clean start. Do not import the old `~/.hapi/hapi.db` history.

## Cut from HAPI (do not build, do not leave hooks for)

Namespaces, a users table, JWT and Telegram login (single bearer token only).
Telegram, Serverchan, FCM, iOS push and any notification channel. Versioned
optimistic updates (`metadata_version`, `agent_state_version`, stale-update
rejection). Socket.IO, WebSockets and the Hypertea socket-transport envelope
(commands are HTTP, events are SSE). Runner handoff, version
skew banners and deployment metadata. Message epochs, truncate, copy-batch,
clear gates, session fork. Importing sessions from other tools. Work graph,
team state, todos and event links. Session export. Usage scanning (a later
phase). First-use hints and decorative UI. The file browser, git diff and
file actions. Global search and session export. Work graph, team state and
todos. Composer extras: skills, slash-command suggestions, `@` mentions, path
autocomplete and collaboration or plan mode. Tool-specific cards beyond a
generic renderer (add later when the generic one hurts).

## Model, effort and stop

The composer toolbar is exactly: attach (later milestone), model selection and
stop.

- Stop interrupts the running turn (driver abort) and works from any device.
  It never leaves a message in `dispatching`; unacknowledged input becomes
  `uncertain`.
- Model selection chooses the model and the effort (thinking level) the model
  supports. Hard-code Claude's catalog for now (opus, sonnet, haiku and the
  efforts each accepts) in one place so it is easy to update. The choice is
  stored per session, applies from the next turn, and is shown in the UI.
  Claude Code 2.1.306 accepts control requests over stdin mid-session, so no
  restart is needed: `set_model` (model name; null or "default" resets) and
  `interrupt` (optional reason) for stop. Claude re-presents the context to
  the new model, so expect a cache miss after a switch. Verify against a real
  Claude Code how effort is changed mid-session (`set_max_thinking_tokens` is
  present in the binary; confirm whether it corresponds to the effort levels)
  and record the finding in an ADR. Use the control channel, not a restart.

## The owner never blocks an agent

Everything is async. There are no permission prompts (YOLO), and agents must
not wait on the owner:

- Disable Claude's interactive tools that block on a person (`AskUserQuestion`,
  plan approval via `ExitPlanMode`). An agent that needs input says so in
  plain text and ends its turn; the owner replies later as an ordinary message.
- Credential sharing is async too: the agent's tool call returns immediately
  with a pending request, the owner submits the secret from any device when
  free, and a message is posted to the session when it is done.
- The only owner-facing request kind is credential sharing (later milestone).

## Sessions are recoverable

Importing from other tools is cut, but a session must always be recoverable:
the store keeps the agent's own session id (for Claude, the id from the
`system` init event). After a server restart, or after the owner reopens an
archived chat, the next message resumes it (`--resume`) and history is intact.
Archive and reopen are supported; delete is not required.

## Multiple devices

The owner uses a phone and a browser, possibly at the same time. This needs
no versioning because the server is the only writer and clients send
commands, never state:

- Every client sees the same event stream. Events carry a per-session
  monotonic sequence number; a reconnecting client resumes from its last
  sequence and receives anything it missed.
- Every client message has a client-generated id (UUID). The server treats a
  repeat of an id as the same message, so a retry from a flaky phone connection
  cannot send twice. This is also what the `uncertain` state relies on.
- Commands for one session are serialized by that session's task. Two devices
  steering at once resolve in arrival order; both messages are kept.
- Composer drafts are per device and live in the browser, not on the server.
- Rename, archive and similar actions are last-write-wins.

## Browser transport

Commands are HTTP POSTs carrying the client message UUID; the reply reports
accepted, rejected, or a typed error. A network failure before a reply leaves
the outcome `uncertain` and is never replayed automatically; the client may
repeat the same UUID safely because the server treats it as the same message.
Command names are listSessions, createSession, sendMessage, stopSession,
archiveSession, reopenSession, resumeSession, retryMessage and setModel.

Events are Server-Sent Events from the Runner's cursor pages: one stream per
open session and one for the session list. The event id is the per-session
sequence, so a reconnect with `Last-Event-ID` resumes with no gap. Send keep-alive
comments, bound buffering, and let a slow reader catch up from SQLite.

## Deployment

`boblm serve` binds `127.0.0.1` only and speaks plain HTTP. TLS and HTTP/2
come from `tailscale serve --https=<port>` in front of it, as the old HAPI
does on this box (`hapi-https.service`), so the credential form gets HTTPS and
the SSE streams share one HTTP/2 connection. No Caddy, no TLS code in the
server. Over plain `http://localhost` the browser uses HTTP/1.1 and allows
about six connections per host; use the tailnet HTTPS address for normal use.

## Workspace layout

```text
crates/tuning          fork-tunable constants: timeouts, limits, Claude launch options (ADR 0018)
crates/protocol        serde wire types: Claude stream-json events, client contract
crates/claude-driver   one Claude Code process over stdio
crates/store           SQLite via Marmot: migrations, typed queries
crates/runner          session tasks, queue and steering state machine
crates/server          HTTP commands, SSE events, auth, static files; the `boblm` binary
crates/live-check      scripted check against a real Claude Code
web/                   Hypertea app
docs/adr/              design decisions
```

## Steering and delivery state machine (port this behavior)

Reference: `/home/user/projects/example/docs/research/claude-steering.md`, and the tests in
`/home/user/projects/example/cli/src/claude/claudeRemote.seam.test.ts` and
`claudeRemoteLauncher.steering.test.ts`.

- A message sent while a turn is running is steered automatically: written to
  Claude with `priority: "now"`. A message sent while idle starts a turn.
- A write is never delivery. Claude must report `command_lifecycle` `started` or `completed`
  for that message's UUID. States per message: `queued`, `dispatching`,
  `started`, `rejected` (back to queued), `uncertain` (held, never replayed).
- `cancelled`, `discarded` and `refused` are rejection. Process exit or a
  missing acknowledgment past the deadline is `uncertain`. After a result,
  an unacknowledged follow-up gets 30 seconds, then the process is stopped and
  the message held as `uncertain`.
- A steer cuts the turn: expect an empty `result`, the original prompt
  `cancelled`, then the steer `started`. Do not treat that as failure.
- Slash commands are never steered; they wait for idle.
- Persist every state change so a restart cannot silently replay or lose input.

## UI requirements (learned the hard way)

- The composer is a plain `<textarea>`. Enter sends, Shift+Enter inserts a
  newline.
- It keeps focus while a send is in flight. Use `readOnly`, never `disabled`.
- The caret stays visible in an empty textarea in Safari. Typing while the list
  streams must never move the caret or drop characters. Use Hypertea's
  uncontrolled field behavior (ADR 0007) unless a controlled field is required.
- The message list appends without losing input, selection, or focus.
  Autoscroll only while within 80 px of the bottom; otherwise preserve reading
  position and show a new messages control.
- Dark only. Calm, dense, readable. No toasts for chat activity. Review desktop
  and phone screenshots with the owner. Safari caret verification is manual
  on Safari; use docs/safari-checklist.md.

## Rules for the implementer

- Run `cargo fmt` before reporting. Use `cargo check -j4` while iterating and
  focused tests; the machine is shared with large Rust builds.
- Rust edition 2024, strict clippy, no `unwrap` on external input. Tolerate
  unknown fields and unknown event types from Claude Code.
- Do not use em dashes in prose or comments.
- Add tests for behavior, especially the state machine, with a fake driver.
- Do not commit, push or install system packages. Leave work in the working
  tree and report.
- Missing product decisions are questions for the owner. Make reasonable,
  reversible choices otherwise and record them in `docs/adr/`.
- Never run a Cargo command with more than `-j4`.
