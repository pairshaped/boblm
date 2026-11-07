# 0002: SQLite storage boundary

Open one Store per database and clone it to share a mutex-protected writer.
Readers have independent read-only connections. WAL lets readers observe the
last committed snapshot while the writer has an open transaction. Operations
are synchronous and belong off Tokio worker threads.

Sessions contain a local working-directory path and an optional provider-neutral `agent_session_id` and an archive flag. Archive/reopen changes neither history nor delivery
state nor the resume ID. There is no machine entity. Messages have globally unique UUIDs and an
atomically allocated sequence within each session. Range reads use half-open
sequence bounds. Timestamps are caller-supplied Unix milliseconds.

Delivery updates compare the expected state in SQL, so a stale update fails.
The database checks the state vocabulary; Runner owns transition eligibility,
retry policy, and recovery. This avoids baking unresolved orchestration policy
into storage. An identical UUID retry returns the accepted message unchanged. Reusing a UUID
with different session, role, or content fails. Timestamps and delivery state
on a retry cannot replace the stored values.

Migration SQL is embedded with include_str at compile time. A build script
collects the numbered migration files, so adding a migration changes the
embedded set automatically. Marmot's public API accepts paths, not in-memory
SQL. Store materializes the embedded files in a temporary directory beside
the database, invokes Marmot, and removes that directory. The installed binary
never reads migrations from the source checkout. The database directory must
be writable, as required for SQLite WAL files too.

Generate bindings with `cargo run -j4 -p boblm-store --example store-generate`; add
`-- --check` to verify them against a disposable database under target.
Generated bindings stay beside their SQL owners. Development generation still
reads the source migration files.

BobLM pins Marmot to git revision `4c0934bed315c81de67e08c60f00a2ad74969c5b`.
This revision bundles SQLite 3.51.3 with the WAL reset corruption fix.
To update Marmot, review the upstream diff, bump the revision, and rerun verification.

Each session has a separate durable event sequence. Message append, delivery
update, and session changes commit their event in the same transaction.
Repeating a message UUID emits no additional event. Readers resume strictly
after their last sequence, in bounded pages. Claude events can be recorded
before publication. These internal event payloads do not define the client
contract. Drafts stay in each browser. There are no version columns, users,
namespaces, or machine identifiers.

Sessions persist model and optional effort, and the model and effort last applied to a provider process. The default model is sonnet, matching
the driver's current default; absent effort delegates to Claude's default.
Changing this choice records an event atomically. The tuning catalog defines supported models and efforts. The runner applies the
stored choice at the next idle turn boundary (ADR 0006).
