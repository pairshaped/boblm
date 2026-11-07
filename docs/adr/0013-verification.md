# 0013: Verification

`bin/check` is the single entry point and the only thing that counts as a pass.
It runs, in order: the Marmot source-tree check (and its behavior test), `cargo fmt --check`, the browser
build for embedded assets, `cargo check` for all targets, the Marmot binding and
browser-contract generation checks (stale generated output fails), strict clippy
with warnings denied, all Rust tests, and
the browser typecheck and tests. It fails on any `warning:` in its output,
including Cargo's own diagnostics that RUSTFLAGS cannot promote.

The browser contract includes the limits it shares with the server, taken from the
tuning crate (ADR 0018), so `bin/check` fails if a tuned contract value is not
regenerated.

All Cargo compilation and test commands use `-j4` and `--workspace`. A package built
alone (`-p`) resolves different dependency features than the workspace, and Cargo then
keeps a second copy of every shared dependency. Measured on this workspace: mixing `-p`
and `--workspace` builds took 32 seconds and 2.5 GB, three copies of tokio; `--workspace`
alone took 18 seconds and 1.7 GB, one copy. Dependencies carry no debug info in the dev
profile, which takes a further quarter off (1.3 GB). Focused runs filter by test name
instead of selecting a package.

`bin/prune-target` bounds `target/`: it removes stale scratch always, and when the
directory is over its limit (6 GB) first the incremental caches, then dependency
artifacts older than a week, then the oldest remaining artifacts until it is under the
limit. Cargo rebuilds anything missing, so this only costs build time. It takes Cargo's
build lock for every profile first, so it never deletes under a running debug or release
build, and `bin/check` runs it at the end. It also counts build-script output, repeats until
hard-linked copies are gone, and refuses a directory that is not a Cargo target. The machine is shared with
very large Rust builds.

Delivery and steering behavior is tested against fake children and fake drivers,
not a real agent. Tests cover acknowledgment, rejection, process exit while
pending, unknown event tolerance, restart recovery, deadlines, concurrent
clients, and idempotency by UUID. Parsing external input never unwraps.

Real-agent checks (the live checks) run only when a bean explicitly calls for
them, in an isolated configuration directory under `target`, with credentials
copied in and removed afterward. Their evidence is recorded under `docs/`.

Browser checks use jsdom for exact text, selection, focus and scroll behavior.
Safari caret and IME behavior, and screenshot review at desktop and phone
widths, are manual checks recorded in `docs/safari-checklist.md`.

Work is not reported done without a passing `bin/check`. Nothing is committed
or pushed without the owner asking.
