# 0029: Page the chat from its latest messages

Opening a chat starts with its latest 50 displayable message records. Tool-only
stretches extend the page to find nearby conversation text within the retained
window's budget. Adjacent tool-only pages leave one base batch available from the
previous window for scroll anchoring. Live events follow the history snapshot's
sequence. Older and newer pages use exclusive event
sequence cursors. Each read uses one SQLite transaction, so messages, current
delivery states and pending credential requests share the returned cursor.
History reads never start an agent.

History queries use partial indexes to skip transport deltas and lifecycle events.
An assistant message stays at its stream-start position. Its final message replaces
the partial content when available; otherwise the server joins that message's text
deltas. Tool names are indexed as events arrive so results retain their names when
the call is outside the page. Existing event history remains the durable source.

Claude's injected continuation summary appears in a closed "Compaction summary"
disclosure using the shared tool-detail renderer. The compaction separator stays
visible. The browser recognizes the provider's fixed summary preamble only in
Claude user events; owner messages and assistant replies remain ordinary text.
The disclosure stays separate from routine tool groups.

The browser folds incoming live events into existing rows. It keeps at most 400
ungrouped rows for the selected chat and releases the previous chat's history on selection.
Wheel, touch, keyboard or scrollbar-drag intent near an edge loads an adjacent
page in that direction. Each gesture is consumed after requesting a page, so
layout changes and anchor restoration cannot start repeated paging. Once the window is full,
loading older messages removes rows from the newer end, and loading newer messages
removes rows from the older end. Live output cannot evict rows while the owner is
reading them. The owner can return directly to the latest page with the overlaid
down arrow.

Before an adjacent page request, the browser records a visible message and its
position inside the scroll area. It restores that position as the page arrives,
including while loading controls change. Session and request identities reject
stale replies after switching chats, locking or starting another request. Page
snapshots do not advance the live cursor on older/newer reads, and row revisions
prevent overlapping snapshot and live updates from duplicating streamed text.

The browser pauses the selected chat's event stream while the document is hidden.
After fifteen minutes without a change in the message area's scroll position,
it clears the selected chat's rendered window, event context and cursor, then
uses ordinary chat selection to load the latest page and follow its bottom.
A hidden document waits until it becomes visible or regains focus before doing
this work. Short absences keep the reading window and resume its live cursor.
Drafts, pending sends and composer attachments survive either path. The fresh
snapshot supplies the live cursor, so hours of missed events are not replayed.
