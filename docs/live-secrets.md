# Live check: an agent asks for a credential

Run on Claude Code with the real `boblm serve`, an isolated `CLAUDE_CONFIG_DIR` and a throwaway
age key under `target/` (removed afterward), Haiku at low effort.

1. Told to use `request_sops_secret` with a stated request ID, `secrets.yaml` and the key
   `["api", "token"]`, Claude called `mcp__boblm__request_sops_secret` and answered ASKED.
2. The chat gained a `secret_requested` event naming the resolved file and key.
3. `submitSecret` with a value replied `saved`. `sops decrypt --extract` returned the value; the
   file, the SQLite database and the server log did not contain it in plain text.
4. The chat received "Credential request <id>: the owner saved the value at api.token in ...
   do not read it back", and Claude acknowledged without asking for or printing the value.
