# 0025: Credential requests

An agent that needs a credential calls the `request_sops_secret` tool with a request ID, a
file and a key path, and never a value. The file must already exist as a SOPS-encrypted
YAML or JSON file; it may be absolute, `~/` for the server user's home folder, or relative
to the chat's working folder. Any such file on the machine is allowed, not only files
inside the chat's project, because a credential may belong in a file outside it (a SOPS file
in the home folder, for example). The key is a path of object keys such as `["db", "password"]`;
missing keys are created, and a path starting with `sops` is refused.

The tool returns at once. It records the request (file, key, expiry), adds a
`secret_requested` event to the chat, and tells the agent to end its turn. The browser shows
a masked input for every request that has no `secret_resolved` event yet, on every device
watching the chat. The owner saves a value or declines; either way BobLM sends the chat one
message saying what happened, under a message ID derived from the request, so a repeat is
harmless. The agent never receives the value.

Requests last ten minutes (`SECRET_REQUEST_TTL`). A chat may have five waiting. Requests are
stored with their state (`pending`, `saving`, `saved`, `canceled`, `expired`, `failed`), so
they survive a restart. The scheduler tick expires unanswered requests and fails a save that
never finished, and tells the chat.

## Handling the value

The value travels in one HTTPS request body to `submitSecret`, is held in memory only while
`sops` runs, and is never stored, logged, put in an event, put in a chat message or returned.
A body that fails to parse is reported without quoting it, and the command body type has no
`Debug`. There is no browser-side encryption: the server is the process that writes the file,
and the connection to it is already encrypted by Tailscale and HTTPS.

Writing is done on a private copy in a mode 0700 folder beside the file:

1. Copy the file, then `sops set` a canary value at the key and check the canary is not in
   the copy in plain text. A file whose encryption rules (`encrypted_regex` and similar)
   would leave the key readable is refused.
2. `sops set` the real value, passed on stdin, never on a command line.
3. Check the original file is still the same file with the same bytes; if not, nothing is
   written and the chat is told to ask again.
4. Give the copy the original's permissions and rename it over the original.

SOPS error text is shown to the owner (the first 300 characters, with the value blanked out)
so a missing key or unreachable key service can be fixed. SOPS and the keys it needs
(`SOPS_AGE_KEY_FILE` or a cloud key service) are the server user's, and `sops` must be on
the server's PATH.

## Claude Platform API account

The Haiku reviewer fallback uses account key `claude-api-a`
(`CLAUDE_ACCOUNT_KEY_API_A`). For each launch, BobLM extracts only
`["ANTHROPIC_API_KEY_A"]` from `~/.config/sops/user-secrets.yaml`, through the
same one-key SOPS reader as OpenAI. `CLAUDE_API_SOPS_FILE` and
`CLAUDE_API_SOPS_KEY_A` define that location. A missing or invalid key fails the
spawn with the file, key path and next step, without revealing the value.

The factory injects the value as `ANTHROPIC_API_KEY` only in the API child's
environment. It never writes the key to a config, argument, event or log. The
API child receives a private `CLAUDE_CONFIG_DIR` under the instance data directory,
selected by `CLAUDE_ACCOUNT_HOMES`. This home has no subscription credentials;
inherited OAuth and provider overrides are removed. Subscription children have
API keys and auth-token overrides removed and keep their subscription config.
Each dev instance gets its own API home as well (ADR 0014).

## Tests

`crates/server/tests/secrets.rs` runs the real `sops` and `age-keygen` with a throwaway key under
`target/`: a saved value decrypts correctly and appears nowhere in the file, events, replies
or chat message; requests are refused before the owner is asked when the file or key is
wrong; a request is answered once; declining and expiry tell the chat; a file whose rules
leave the key in plain text, and one encrypted for someone else, are left untouched with a
reason; and an unparseable body never echoes the value. Both tools must be installed to run
the test suite.
