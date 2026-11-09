# Run BobLM

Run `boblm init`, then `boblm serve`. Init prints the bearer token once and the
URL to open. Enter that token in the browser. Keep the listener on 127.0.0.1;
use Tailscale Serve HTTPS for access from your other devices.

Init creates `$XDG_CONFIG_HOME/boblm/boblm.json`, or
`~/.config/boblm/boblm.json` when XDG_CONFIG_HOME is unset. The file has mode
0600. The database defaults to `$XDG_DATA_HOME/boblm/boblm.sqlite`, or
`$HOME/.local/share/boblm/boblm.sqlite`. Relative XDG paths are ignored. Serve
creates the database directory when needed and listens on port 3000.

Use `boblm init --config PATH` and `boblm serve --config PATH` for a different
config location. Init never overwrites an existing file, including a symlink.
To use an existing configuration, run serve with its path and set its file mode
to 0600. Serve refuses a config that grants group or other access.

The browser assets are embedded in the binary. A `webRoot` field in the config
can override them during development. Keep the config outside that directory
because its files are public.

Init checks the existing Tailscale Serve configuration for an HTTPS root proxy
to its localhost port. If found, it prints that URL; otherwise it prints the
localhost URL. It does not configure Tailscale. For normal use, put Tailscale
Serve HTTPS and HTTP/2 in front of the listener. Plain localhost HTTP uses the
browser's limited pool of HTTP/1.1 connections. Never expose the listener
directly to the network.

## Restart one chat's runner

Use this after installing new runner code or changing agent launch settings:

```sh
boblm restart-runner --session CHAT_ID
```

BobLM waits up to 30 minutes for the latest activity event to be a turn result,
followed by 20 seconds of silence, with no turn, background jobs, queued input,
or unconfirmed input. Turn starts count as activity; usage polls and job snapshots
do not reset the silence clock. Use `--wait SECONDS` to change the maximum wait and
`--config PATH` to select a different instance. A timeout leaves the runner
running. Other chats keep working.

The command stops the quiet runner's unit. The web server starts its replacement
when it reconnects. A quiet shutdown lets later messages dispatch without Resume.
An interrupted turn, unconfirmed input, a crash, or an explicit owner Stop keeps
the queue held. A runner running older code must be replaced once before it can
record a quiet shutdown.

## Add a second Claude account

Plan A keeps your existing Claude login. To add plan B with the default database
location, run:

```sh
install -d -m 700 "$HOME/.local/share/boblm/claude-plan-b"
CLAUDE_CONFIG_DIR="$HOME/.local/share/boblm/claude-plan-b" claude
```

Then enter `/login` in Claude Code and sign into the second plan account. Exit
Claude Code after login. For a custom database location, use `claude-plan-b`
beside that database. For a dev instance named `accounts`, the home is
`$HOME/.local/share/boblm-dev/accounts/data/claude-plan-b`.

Add `CLAUDE_ACCOUNT_KEY_PLAN_B` to `CLAUDE_PLAN_ACCOUNTS` in
`crates/tuning/src/lib.rs`. Rebuild and restart the web server, then safely
replace existing runners using the command above. Keep A in the list.
BobLM chooses accounts automatically and drains the account whose weekly reset
comes first. At 2% left, idle chats can hand off. A chat with a running background
job waits until that job ends. You can still create an owner chat when every
account is past the cutoff; BobLM uses the account whose reset comes first.
With A alone, owner chats and explicit-model peers keep using A at any usage.
Role spawns continue to enforce their allowance cutoff.

API B needs no `/login`. Save its key at `["ANTHROPIC_API_KEY_B"]` in
`~/.config/sops/user-secrets.yaml`, then add `CLAUDE_ACCOUNT_KEY_API_B` to
`CLAUDE_API_ACCOUNTS` and rebuild. Haiku reviewers try A before B, skipping an
account whose credit is exhausted. API keys reach children through their
environment and are never written into the Claude config homes.

## Build from source

Install browser dependencies and build the assets before compiling the server:

```sh
npm ci --prefix web --ignore-scripts --legacy-peer-deps
npm run --prefix web build
cargo run -j4 -p boblm-server --bin boblm -- init
cargo run -j4 -p boblm-server --bin boblm -- serve
```

Hypertea uses the already built local package from the local checkout. Rebuild
the browser assets after changing browser code and before compiling the server.
`bin/check` builds the browser before checking Rust so the embedded assets match
the source. A deployed binary needs neither checkout nor web asset directory.

## API

The API accepts POST /commands/{name} with JSON bodies and Authorization:
Bearer TOKEN. Event streams use GET /sessions/events and
GET /sessions/{sessionId}/events with the same header. Last-Event-ID is the
global list sequence on the first route and the session sequence on the second.
Attachments upload with PUT /sessions/{sessionId}/uploads/{uploadId} (raw body,
X-Filename percent-encoded, up to MAX_UPLOAD_BYTES, 50 MiB) and are fetched with GET on the same
path; sendMessage names them in `attachments` (ADR 0017). Uploads are stored under
`uploads/` beside the database. No command is automatically replayed after a lost reply. A deliberate message
retry must retain its original UUID.
