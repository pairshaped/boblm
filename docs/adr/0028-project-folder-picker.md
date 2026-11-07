# 0028: Choose a project folder inside home

The folder button at the top right of the sidebar opens the project browser.
It starts at the service user's HOME directory. Clicking a folder navigates into
it; the [..] button goes to its parent and is disabled at home. Use this folder
adds the displayed folder through the idempotent addProject command and closes
the browser. The sidebar has no path input or separate add-project form.
No folder is created and no file content is read by the picker.

The authenticated listDirectories command accepts an optional absolute path and
returns the canonical path, its parent (null at home), and child folder names and
paths. Omitted or null path means home. Canonical paths must stay within canonical
HOME, including symlink targets. Links outside home, broken links, files and names
that cannot be represented as UTF-8 are omitted. Dot-prefixed entries are hidden.
Names are sorted without case first, then by their original spelling.

Filesystem work runs on a blocking worker. MAX_DIRECTORY_ENTRIES in the tuning
crate bounds the scan; the serialized reply must also fit MAX_BODY_BYTES. A folder
that exceeds either limit returns an error asking the owner to choose another
folder or reduce its entry count. The
picker is a navigation boundary, not a sandbox against other local processes
changing directories while a request runs. The addProject API can accept
folders outside home, but the browser only offers folders inside home.

The browser keeps the last successful listing while a navigation request is
pending. It disables selection during loading. A failed navigation preserves that
listing and shows the cause with a next step. Each navigation has a browser-only
request identity; older replies and replies after cancel or lock cannot change a
newer picker. Requests are never retried automatically. The contract decoder
validates directory replies before the view receives them.

Opening the picker moves keyboard focus inside it. Tab navigation stays within
its buttons; Escape and Cancel close it and return focus to Browse folders.
Use this folder returns focus to Browse folders while the add-project command
runs. Failures appear as errors in the main view and are never retried automatically.
