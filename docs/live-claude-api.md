# Claude API reviewer check

On November 9, 2026, one real Claude request ran through the reviewer role in an
isolated `bin/dev-instance` on port 3029. Its data, provider config and temporary
launch wrapper were under `target/model-routing-live`, separate from the live
BobLM instance. The dev database marked the Luna API step exhausted to select
the Haiku fallback. No production chat or database was used.

The routing receipt selected `reviewer-claude-api`, model `haiku`, high effort,
account kind `claude-api` and stored account key `claude-api-a`.

The wrapper retained only an allowlist of authentication and model metadata:

- The child had `ANTHROPIC_API_KEY`, no OAuth token and no subscription credential
  file in its config directory. The key's value was never captured or printed.
- `CLAUDE_CONFIG_DIR` was the dev instance's `data/claude-api-a` directory.
- Claude's initialization reported `apiKeySource: ANTHROPIC_API_KEY` and model
  `claude-haiku-5-5`.
- The assistant and result both reported `claude-haiku-5-5`. The assistant replied
  `API_HAIKU_OK`, and the result reported success and a cost of $0.002783075.

The check used the factory's one-key SOPS extraction of `ANTHROPIC_API_KEY_A`.
The test chats were archived and deleted, and the dev instance and browser
session were stopped after verification. The temporary config and launch wrapper
were removed. No subscription credentials were copied into the API home.
