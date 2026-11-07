# 0001: Application stack

BobLM uses a fixed-port web process and independently supervised per-chat runners
(ADR 0040). Port 0 instances keep runners in process. Tokio drives Claude, Codex
and Pi through provider drivers. Rust crates use edition 2024.
Values a fork is likely to change live in the dependency-free `crates/tuning`
crate and in `web/src/theme.css` (ADR 0018); other crates cite them and never
repeat the literal.

SQLite stores sessions and messages. Marmot owns forward migrations and
produces typed rusqlite queries from SQL beside each Rust module. SQLite is
bundled. Marmot uses a git dependency pinned by revision.

Rust serde types define the wire contract. Generated TypeScript types and
decoders serve the browser, with Rust JSON fixtures parsed by TypeScript tests
to catch drift. Hypertea provides the browser application. Commands use HTTP
POST and events use Server-Sent Events through an app-level subscription. The UI is dark only and uses a plain textarea with
uncontrolled field behavior to preserve focus and caret position while streaming.

The runner persists every delivery transition. It treats Claude's
`started` or `completed` acknowledgment for the message UUID as delivery. Uncertain messages
remain held and are never replayed automatically. Slash commands wait for idle.

Claude, Codex and Pi run with permissions bypassed. BobLM does not import HAPI history.
