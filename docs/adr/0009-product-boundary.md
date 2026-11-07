# 0009: Product boundary

BobLM is a personal tool: one owner, one machine, one set of fixed behaviors.
It supervises coding-agent sessions and nothing else.

There is no machine entity. Sessions have a working directory that is a plain
absolute path on this host. There is no machine registry, remote runner,
per-machine setting, or machine identifier anywhere in storage or the contract.

There is one account. Access is a single bearer token (ADR 0008). There are no
users, namespaces, roles, or login flows.

Behavior is fixed, not configured. A fork that wants different behavior changes
code, in the tuning crate and theme file where that is easy (ADR 0018); there are
still no settings screens. Sessions run with permissions bypassed. The
interface is always dark with one font size. There are no settings screens, no
theme or layout options, and no preferences stored on the server. Model and effort are choices stored
with each session (ADR 0006).

Out of scope, and not designed for: a terminal, voice dictation or assistants,
a file browser or git view, global
search, session export, importing sessions from other tools, session fork,
a work graph or todos, first-use hints, and localization. Web Push notifications (ADR 0034) and the `usage_report` tool (ADR 0042)
are implemented. Features are added when the owner needs them, not left as
hooks. Telegram and other chat integrations are not designed for.

The owner reaches the app from a laptop browser and a phone, usually both at
once (ADR 0011).
