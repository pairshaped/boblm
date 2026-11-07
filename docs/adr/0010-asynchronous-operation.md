# 0010: The owner never blocks an agent

An agent never waits for the owner. Work continues while the owner is busy in
another chat or away from the screen.

Sessions run with permissions bypassed, so there are no permission prompts.
Claude's interactive tools that block on a person (the question tool and plan
approval) are disabled. An agent that needs input can use `ask` to create a durable owner question
(ADR 0033), then end its turn. The owner answers asynchronously from any device.

Anything else that needs the owner is a pending request. The agent's tool call
returns immediately with a pending status. The owner resolves the request from
any device when free, and a message is posted to the session when it is done,
so the agent can continue. The first pending request kind is credential sharing:
the owner pastes a secret into a masked field, the browser encrypts it for the
local helper, and the server relays only ciphertext; plaintext never appears in
chat, event payloads or logs.

Messages sent while a turn is running steer that turn automatically at the next
tool boundary (ADR 0004). There is no queue-or-steer choice. Slash commands and
messages that cannot steer wait for idle without blocking the sender.

Stop and archive are the owner's only forceful controls, and both are explicit.

Agents run as the owner with permissions bypassed, so they can read the owner's
files, including the configuration and its bearer token. Per-session MCP
capabilities (ADR 0015) bind an agent's identity for peer tools and limit
accidents; they are not a privilege boundary against an agent that chooses to
misbehave. Agents are trusted to the same degree as the owner's shell.
