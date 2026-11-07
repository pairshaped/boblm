# 0008: Local HTTP application supervisor

For fixed-port instances, App holds runner proxies per persisted session ID.
User systemd units own and supervise the delivery actors and provider processes
(ADR 0040). Runner startup recovers delivery state without starting providers.
Port 0 instances use in-process actors. Creation
holds the registry lock across insertion and Runner startup, so competing
requests cannot create duplicate tasks. Existing IDs are idempotent for the
same directory. Commands use /commands/{commandName}; SSE uses /sessions/events
and /sessions/{id}/events. A repeated send UUID always replies accepted when its
content matches, even when the original request already dispatched it.

Commands and SSE require a single configured bearer token. Token comparison
uses subtle's constant-time byte equality. Static browser files are public;
the token is never embedded in those files. The browser supplies it when
calling the API. There are no cross-origin API permissions. Request bodies
are bounded at MAX_BODY_BYTES (1 MiB) and parsed through the protocol validators.

`boblm serve` reads a JSON configuration file, defaults to `$XDG_CONFIG_HOME/boblm.json`
(or `~/.config/boblm.json` when XDG_CONFIG_HOME is unset), and accepts
--config PATH. Configuration contains token, database, port and webRoot. Paths
are relative to the current working directory. Tokens must have at least 16
non-whitespace bytes. Working directories must be existing absolute host paths.
The listener binds only 127.0.0.1. Tailscale Serve supplies HTTPS and HTTP/2;
there is no application TLS implementation or remote-runner listener.

SIGINT or SIGTERM stops accepting HTTP work, drops SSE bodies, and detaches
runner proxies. Fixed-port runners and their providers keep running. In-process
actors shut down with the web process. HTTP graceful draining alone
could wait forever for open event streams. Store and Runner remain separate
crates; the HTTP application owns their lifetimes.
