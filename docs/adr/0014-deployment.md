# 0014: Local deployment and configuration

`boblm serve` listens on 127.0.0.1 over plain HTTP. TLS and HTTP/2 come from
`tailscale serve` on its own HTTPS port in front of it, the same arrangement the
previous system used on this machine. The server holds no TLS code and never
listens on a non-loopback address. HTTP/2 matters because every open session
holds an event stream; over plain localhost HTTP/1.1 a browser allows about six
connections per host, so normal use goes through the tailnet address.

The browser app is installed as a Dock app on the Mac (Safari's Add to Dock
works for any site) and added to the home screen on the phone.

Instance configuration is one JSON file holding the bearer token, database path, port and
web asset location. `boblm init` creates it: a token of at least 32 bytes from
the operating system's random source, file mode 0600, no overwrite of an existing
file, a database path in the user's data directory, and a printed URL. `boblm
serve` refuses or warns when the file is readable by group or others. The token
is entered once per browser and kept in local storage; a Lock action clears it
and disconnects that browser's streams without stopping agents.

The built web assets are embedded in the binary so a single file runs the app,
and database migrations are embedded likewise. Releases build with a thin-LTO,
stripped profile and keep unwinding enabled so a panic in one session task does
not end the process.

Tunable behavior (timeouts, limits, Claude launch options) is not in that file;
it lives in the tuning crate and changes with a rebuild (ADR 0018).

A systemd unit runs `boblm serve` and a second unit runs `tailscale serve` in
front of it. Session runners live in separate user systemd units (ADR 0040).
Restarting the web server detaches its clients and leaves provider processes,
subagents and background jobs running. Each unit keeps `KillMode=control-group`:
stopping a runner or losing its process clears that runner's descendants.

`bin/restart` checks the authenticated runtime-capability reply against the web
unit's actual MainPID. A matching persistent-runner server can restart while
agents are working. Missing capabilities, a mismatched PID or a legacy server
fall back to the idle guard, which checks twice before restart. `--wait SECONDS`
waits for legacy chats to become idle; `--force` explicitly interrupts a legacy
server's work; `--detach SECONDS` schedules the same command separately. The
first rollout from a legacy release still stops the agents it owns.

Work on BobLM itself runs against a separate dev instance, not the real server, so testing never targets real chats. `bin/dev-instance start|restart|stop|status|token` builds
the web assets and the server, then runs `boblm serve` with its own config: its own port
(3017 by default), token and database under `$HOME/.local/share/boblm-dev/NAME`, never the live
database. `--copy-db` seeds it once from a read-only backup of the live chats; a chat resumed
in both places would drive one agent session twice, so use it sparingly. The config sets
`webRoot` to `web/dist`, so a web-only change needs `npm run build` and no restart. `restart`
builds before replacing the web process and keeps its session runners alive.
`stop` also stops that dev database's runner units, so explicit cleanup leaves no
agent descendants. Other instances have different unit prefixes. It listens on
127.0.0.1 only; the script prints the `tailscale serve` command to reach it from another
device and does not run it. `bin/test-dev-instance` covers the script with a fake server.

The server shrinks what it sends. The embedded browser files carry a content hash and a
gzip copy, so a reload costs a `304 Not Modified` instead of the whole script. JSON replies
to commands, such as the chat list and chat history, are gzipped when the browser accepts it
and the reply is over 1 KB. Event streams and uploaded files are never compressed: a stream
must reach the browser as it is produced. This matters because the owner's devices reach the
server over a tailnet, about 100 ms away, where bytes and round trips are what a reload feels.
The browser also remembers the last chat list and last open chat in local storage and shows
them at once while the fresh list loads.
