# 0023: Projects and agents per chat

## Projects

A project is a base directory the owner adds once. The sidebar lists projects, and each
project lists its chats. A chat is always created inside a project and works in that
project's directory, so no screen ever asks for a folder when starting a chat.

`addProject` takes a client-generated project ID, an absolute path to a directory that exists
on this machine, and an optional name (default: the last part of the path). The path is resolved
(`.`, `..`, doubled and trailing slashes, symlinks) before it is stored, so two spellings of one
folder are one project, and it is unique; the same ID with the same path is the same project, and
an ID or path already used for something else fails with a message saying so. `removeProject`
succeeds only for a project with no active (unarchived) chats; otherwise it says how many keep
it and asks the owner to archive them first. Any archived chats go with the project: each is
deleted exactly as a single archived chat is (ADR 0030), and nothing is deleted unless every
chat is archived. The browser asks for confirmation first. A failure part way leaves the rest in
place for a retry. Removing a project that is already gone succeeds, so a retry after a lost reply is harmless. Another device sees a changed project when its chat
list next refreshes (on reload, or when its tab returns to the foreground), since projects are
not part of the durable list stream. The database refuses a chat without a project.

Agents get the same view: `list_projects`, and `spawn_peer` takes an optional `projectId`
(default: the caller's own project) in place of a working directory (ADR 0015).

## Agents and models

A chat runs on Claude Code, Codex or Pi. Its model decides which: the catalog (the `MODELS` list in
the tuning crate, ADR 0018) names every model the server offers, the agent that runs it and the
reasoning efforts it accepts. Creating a chat picks a model and optionally an effort; the server
checks both against the catalog and records the agent. The agent never changes for the life of a
chat, so switching model inside a chat is limited to models of the same agent, and the message
says to start a new chat to use the other agent. The catalog's Codex entries match what
`codex app-server` reports through `model/list`, and a live check compares them.

The New chat dialog, opened from a project, asks for the model (one list, grouped by agent) and
the reasoning effort that model supports. The model defaults to the last one chosen in this
browser, otherwise `DEFAULT_MODEL`.

## Codex

Each Codex chat runs its own `codex app-server` process group and speaks JSON-RPC over stdio
(docs/live-codex-protocol.md). The driver turns Codex's notifications into the Claude-shaped
events the runner already consumes, which is why storage, the delivery state machine and the chat
are shared. Events keep the stored kind `agent_event`; their payloads have Claude's shape for every
agent.

Delivery follows ADR 0004. A message is consumed when Codex announces it as a user message item
carrying the ID BobLM gave it; an accepted request alone proves nothing. A steer goes to the running
turn. After an explicit stale-turn refusal, the driver reads the latest turn with
`thread/turns/list` and retries once with the same message UUID. If that turn has
ended, it starts a new one. Failed or malformed lookups and further refusals return
the message to the runner's rejection handling. A timeout on a message request
remains uncertain and is never replayed automatically. Child-thread notifications
cannot change the parent chat's turn or delivery state.

A final refusal is shown in the chat with its reason. Interrupt ends the turn with
its own result. Codex's steer does not cut a running tool, unlike Claude's.

Slash commands: `/compact` runs Codex compaction and shows the same marker; `/clear` starts a new
Codex thread, which reports a new conversation ID and shows the same marker (ADR 0019). Resume
continues the stored thread, and a failed resume is an error that says no new conversation was
started in its place.

Codex runs unattended, as Claude does (ADR 0010): approvals never, the sandbox off, the same
instruction to say what it needs in plain text, and any request that would wait for the owner is
declined. A chat with no Codex account key uses the owner's normal Codex home and configuration,
with only those settings overridden. The peer tools are given to it as an HTTP MCP server whose
capability travels in an environment variable, never an argument (ADR 0015).

Codex chats may select a `codex-plan` or `codex-api` account key. `CODEX_ACCOUNT_HOMES` maps
each key to a CODEX_HOME; the AgentFactory sets that home on the child. The API home is
BobLM-owned: its `config.toml` defines a provider with `env_key = "OPENAI_API_KEY"` and the
Responses wire API. The key is extracted from SOPS for each spawn and placed only in the child's
environment. It is never written to auth.json, a file, or an argument. The API account uses the
stable BobLM-assigned identity `openai-api`, and missing rate-limit or credit data is tolerated.
A missing key refuses that spawn with a plain sentence naming the SOPS path and the next step.

The API home is prepared under an exclusive file lock, and its config is replaced atomically,
so two children starting together cannot read a half-written file. Codex startup on that home
holds the same lock through the initialize handshake. The first Luna and Sol children therefore
serialize their one-time state initialization instead of racing on a brand-new home. The lock is
advisory and local to the BobLM-owned home; the owner's normal Codex home is not locked.

## Storage

Sessions persist `project_id`, `agent` and the provider-neutral `agent_session_id`.
The resume ID belongs to the chat's provider. Provider events use `agent_event`,
and conversation changes use `agent_session_changed`.

## Model catalog and labels

`MODELS` in `crates/tuning/src/lib.rs` offers exactly seven models, always the newest of
each: opus, sonnet, haiku, sol (`gpt-6.1-sol`), terra (`gpt-6.1-terra`),
luna (`gpt-6-luna`) and Pi-backed dsf
(`deepseek-ai/DeepSeek-V4.1-Flash-1017`). Each entry has a short `label` for tight places:
the model selectors, the drawer header and tool text. Tool text shows `label (id)` where there
is room. Stored settings, routing state, Pi's models.json and every `model` argument keep
the full ID.

A newer version replaces the ID in tuning and nothing else; the role table points at the same
named constants. A chat whose stored model is no longer offered keeps launching, resuming and
archiving, because only `check_choice` (create, switch and spawn) reads the catalog. Switching
to a model outside the catalog is refused.
