# Sidebar ordering browser check

Checked on 2026-10-23 with the isolated `browser_fixture` server, a disposable
SQLite database under `target/ordering-browser`, and the named Playwright CLI
session `boblm-yw1w`. The fixture used fake drivers, three projects and five chats.
No model calls or owner services were involved.

The installed headless Chromium executable was selected explicitly in the session
configuration. Native `playwright-cli drag` moved Alpha below Beta and moved the
first Alpha chat below its third chat. Reloading preserved both orders. Further
`run-code` checks used Playwright's native `dragTo`, with explicit source and target
positions, to assert the final order and persistence against the finished code.

Delayed HTTP responses were exercised in Chromium:

- A list snapshot requested before a drag was held until the save and its fresh
  list completed. Releasing the old snapshot did not change the saved order.
- A rejected reorder started a recovery refresh. Releasing a held pre-drag list
  did not re-enable dragging. Releasing the recovery list restored the server's
  order, re-enabled dragging and kept the rejection visible.

The fixture server was then stopped. A native project drag produced exactly one
`reorderProjects` request and one `listSessions` request. The page reported that
saving had no usable reply and that the list could not be refreshed. It did not
repeat the reorder or refresh automatically.

The same fixture also displayed the Pi agent label, both configured Fireworks
DeepSeek V4.1 Flash model options, and Default/off/minimal/low/medium/high efforts.
Selecting the other model completed a `setModel` request through the fake driver.
This verifies browser wiring, not a provider call.

The ordering test file has 11 tests. Nine mutations in a disposable source copy
failed assertions: ignoring the after edge, allowing cross-project drops,
duplicating the first chat across held slots, sending an unchanged order, using an
unsupported dragover binding name, starting drags from project buttons, omitting
the refresh, accepting an old list reply, and accepting an old reorder reply.
The source copy was removed after the checks.

The independent Luna review checked move semantics, request bodies, recovery,
concurrent list events, lock/unlock reply handling, native bindings and shortcut
order. Its confirmed response-ordering findings have regression tests. Its final
report found no remaining issue in that scope.

Only Chromium was exercised live. Firefox and Safari were not checked. Touch and
keyboard reordering are outside the feature's scope (ADR 0027).

Cleanup: the fixture process exited after SIGINT, `pgrep` found no remaining
fixture process, and `playwright-cli list` showed no open browsers after closing
`boblm-yw1w`. The isolated database, browser logs, configuration and scripts were
removed from `target/`.

Final verification: `cargo fmt` completed; `bin/check` exited 0 and printed
`check passed`, including all 81 web tests, workspace Rust tests, clippy, generated
bindings and contract checks.
