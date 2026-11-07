# Project folder picker verification

Checked on 2026-10-23 with an isolated browser fixture, database and HOME under
`target/directory-browser`. The fixture used fake drivers. No real model calls,
credentials or owner data were involved.

The named Playwright CLI session `boblm-dqi1` used the installed headless Chromium.
Native browser actions opened Browse folders at home, entered projects and an
empty subfolder, selected it, and added the project. The sidebar updated immediately
and retained the project after reload. At 390 by 844 pixels, the picker fit within
the viewport. Home had no parent and Up was disabled there.

The keyboard check confirmed that focus enters the picker, Tab and Shift+Tab wrap
inside it, and Escape closes it and returns focus to Browse folders. An intercepted
Home request returned a folder-permissions error; the previous folder list stayed
visible. A native project drag then confirmed that a successful reorder produced
no error and changed the displayed order.

The browser check exposed incorrect reply decoding for existing project commands.
The HTTP boundary now decodes addProject as ProjectInfo and removeProject,
reorderProjects and reorderChats as Empty. The regression test reproduced the
previous uncertain result before the fix, then passed. The independent Luna review
also identified the Home-navigation failure; its regression test failed before the
fix and passed afterward.

The authenticated HTTP tests cover folders only, hidden folders, home navigation,
internal and external symlinks, missing folders, regular files, invalid paths, the
scan limit and authentication. Web tests cover selection, navigation failure,
late replies, cancel/reopen and lock, rendering and command reply decoding.
Five mutations in a disposable source copy failed assertions: losing the previous
Home listing, accepting stale replies, allowing selection during loading, decoding
a project reply as chat status, and decoding an empty reply as chat status.

`cargo fmt`, `cargo check -j4 --workspace`, focused tests and web typecheck passed.
Final `bin/check` exited 0 with `check passed`, including 86 web tests. The browser
session and fixture were stopped, and their disposable state and logs were removed.
Firefox and Safari were not exercised.
