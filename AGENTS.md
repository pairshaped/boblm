# Working in BobLM

Read `CONTEXT.md` for the project vocabulary. Read `docs/milestone-1.md` before
implementation. It defines the stack, crate boundaries, delivery state machine, and
UI requirements.

- Use Rust edition 2024 and strict clippy. Tolerate unknown Claude fields and
  event types. Never unwrap external input.
- Use `cargo check -j4 --workspace` at verification checkpoints and focused tests for
  behavior. Every Cargo compilation or test command must use `-j4`. Run `cargo fmt`
  before reporting and `bin/check` for workspace verification.
- Test delivery and steering with fake children. A write is never delivery;
  a lifecycle `started` or `completed` acknowledgment proves consumption.
- Use Marmot via its pinned git dependency. Read its
  README before changing storage. Keep SQL beside its Rust owner and use
  Marmot migrations and generated typed bindings.
- Make reasonable, reversible choices and record design decisions in
  `docs/adr/`. Ask the owner about missing product decisions.
- Commit and push only when the owner asks. Leave other changes in the working tree
  and report verification. Do not install system packages or use em dashes in prose or comments.
- Run real Claude checks only when the assigned Bean explicitly calls for it.
- Test changes to BobLM against `bin/dev-instance` (ADR 0014).
- The credential tests (`crates/server/tests/secrets.rs`) run the real `sops` and
  `age-keygen`; both must be on PATH.

## Deployment

The owner gives standing authorization to deploy verified changes; no separate confirmation is
needed. A web-only restart (persistent runners confirmed by `bin/restart`) does not wait for
other chats, because their runners and agents keep working. A change that needs runners
replaced still waits until the affected chats are idle.

Use `bin/restart` from a separate background job after replying, then verify the running release.
It permits web-only restarts when the running service confirms persistent runner ownership;
legacy releases retain the idle guard because their restart stops agent processes. The first
rollout of persistent runners still needs that legacy transition. Explicit runner shutdown
interrupts that runner's agents and jobs. See ADR 0040 before changing runner supervision.
When the owner requests a page reload, request it only after deployment verification.

## Error messages

Write every error as a plain-English sentence: what happened, why if known, what to do next.
Include the technical cause, never a stack trace or debug dump (ADR 0021).

## Where to change behavior

- Timeouts, limits, the appended system prompt, Claude launch options, default
  port: `crates/tuning/src/lib.rs`. Add every new tunable there as one documented
  constant, alphabetized, and never repeat its literal elsewhere. After changing a
  contract value, run `cargo run -j4 -p boblm-protocol --example protocol-generate`.
- Colors and base font sizes: `web/src/theme.css`.
- Models, their agents and efforts: `MODELS` in `crates/tuning/src/lib.rs`, then regenerate (ADR 0023).
- No settings screens. See ADR 0018.

## Build directory

Always run `cargo check`, `build` and `test` with `--workspace`, never `-p` (`cargo run --example` needs `-p`). A package built alone gets different
dependency features than the workspace, so Cargo keeps a second copy of every shared
dependency; mixing the two roughly doubles `target/` and the build time (measured, ADR 0013).
For a focused test, filter by name: `cargo test -j4 --workspace <test name>`.

`target/` is bounded by `bin/prune-target` (limit set at the top of that script).
`bin/check` runs it at the end and it prints nothing while under the limit. Run it by
hand any time, or `bin/prune-target --report` for sizes.
