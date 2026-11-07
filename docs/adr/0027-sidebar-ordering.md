# 0027: Dragging projects and chats into order

Project headers move among projects. Chat rows move only among chats in the same
project. The upper half of a target means before; the lower half means after.
Dropping moves the source into that position and shifts intervening items. Dropping
on itself or into its current position sends nothing. Only native browser dragging
is supported; touch gestures, keyboard reordering and cross-project moves are out
of scope.

Each project and chat has a stored position. New items take the highest position
in their list plus one. The server assigns the named items' existing sorted positions
to their requested order, leaving unnamed items in place. Chat positions belong to
their project. The browser keeps the server's array order; when moving chats, it
permutes that project's entries among their existing slots in the sessions array.

The model holds the dragged item and target for rendering. Native handlers set
text/plain drag data and effectAllowed=move for browser compatibility, but drop
validation uses model state. A drop must match the dragged kind and, for chats,
project. New chat and Remove buttons cannot start project drags. A line marks the
accepted target edge; leaving the target or ending the drag clears it.

A valid drop updates the sidebar immediately and sends reorderProjects with all
project IDs, or reorderChats with the project ID and its chat IDs. Only one reorder
is allowed in flight. Each reply triggers one listSessions refresh so concurrent
creations and renames are not lost. Rejected or uncertain replies also show their
cause; a reorder is never retried automatically. Dragging remains disabled until
that refresh settles. Reorder and list effects and replies carry a browser-only
generation, advanced when a move starts, when its reply arrives, and when the owner
locks the page. Replies from an older generation cannot overwrite the new order,
finish its refresh, or finish a later move after unlocking. A failed refresh leaves
an error visible and allows a new deliberate action. Successful refreshes preserve the reorder error so it does
not disappear before the owner can read it.

Fold state stays keyed by project ID. Shortcut numbers are derived from the current
visible project and chat order. Other tabs learn order changes on their next list
refresh, including when they return to the foreground; reorder events are not
broadcast through the live stream.
