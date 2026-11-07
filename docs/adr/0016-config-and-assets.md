# 0016: First-run config and embedded browser build

`boblm init` and `boblm serve` use the same XDG config path by default, with
`--config` for an explicit path. Init creates a 32-byte OS-random token encoded
as unpadded base64url, writes a private temporary file, then publishes it without
replacing an existing entry. Serve checks the opened config's permissions before
reading its token or opening SQLite. Database directories are created at serve
time, so init does not start or recover sessions. Serve creates a missing database
file with mode 0600 and refuses an existing one that group or others can access;
SQLite gives its WAL files the database's mode, so conversation history is never
group or world readable.

Per-session agent files live in `$XDG_RUNTIME_DIR/boblm`, or in `run` beside the
database when that is unset. The directory must be mode 0700 (it is created that
way, and an existing one with wider access is refused), files are 0600, and files
left by a crashed server are removed at startup, after serve has checked that the
directory is private (it refuses a wider one before deleting anything). One server owns the directory.

The three browser build outputs are compilation inputs embedded with
`include_bytes!`. Build the browser before compiling the server; `bin/check`
enforces that order. This keeps deployment to one binary without asking Cargo
to install or build JavaScript dependencies. An optional `webRoot` supports
local development. Init omits it, so a new config works without a checkout.

Tailscale URL discovery is read-only, bounded to TAILSCALE_DISCOVERY_TIMEOUT (two seconds), and accepts only
an HTTPS root proxy to this listener's port. Discovery failure uses the
localhost URL. Init never changes Tailscale configuration.
