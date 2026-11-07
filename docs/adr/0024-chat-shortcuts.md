# 0024: Chat shortcuts and folding projects

Each project in the sidebar can be folded shut. The folded set is remembered in the
browser. The chats showing in the sidebar, project by project in sidebar order, are
numbered; the first nine show their number beside their name.

Command plus a digit (Alt plus a digit off a Mac) selects the chat with that number
and moves the cursor into its composer. A bare digit is typing, Ctrl plus a digit is
left to the browser's tab switching, and a held key does not repeat. A number with no
chat does nothing. Selecting a chat any other way also moves the cursor to the
composer; the composer for a newly selected chat is drawn after the selection, so the
focus step waits for it.

Browsers keep Command plus a digit for their own tabs, so the shortcut works in an
installed web app window (Safari "Add to Dock", Chrome "Install as app"), not in an
ordinary tab on macOS.

Creating a chat in a folded project unfolds it so the new chat shows.
