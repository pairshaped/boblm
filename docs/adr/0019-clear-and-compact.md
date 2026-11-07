# 0019: Clear and compact

Claude, Codex and Pi implement `/clear` and `/compact`, typed into the composer
like any message. The toolbar includes model and effort selection; clickable
context meters send `/compact`.
They follow the slash-command rules of ADR 0004: they never steer, they wait for
the running turn and for every outstanding acknowledgment, and then dispatch as an
ordinary turn.

`/compact` keeps the Claude conversation ID. Claude summarizes earlier turns and emits
a `compact_boundary` system event, which the chat shows as a marker where the
compaction happened. Optional instructions after the command pass through unchanged.

`/clear` makes Claude start a new conversation in the same process. Claude reports a new
conversation ID in its next init event. The chat is not forked or replaced: its messages,
attachments and events stay and remain scrollable, and only what Claude sees changes. The
store records the replacement in a `agent_session_changed` event that carries the ID it
replaced (`previous`, null for the first ID). A repeat of the current ID records nothing,
since Claude reports the ID on every turn. The chat shows a marker for each change that has
a predecessor. Because the runner persists the newest ID, a restart or reopen resumes the
cleared conversation, not the one before it; a test starts a new process after a
clear and checks which conversation ID it is given.

Markers are derived from the stored events when the chat is drawn; nothing about them is
stored separately and no extra command exists. A message in the chat before a marker is
history only.

Verified live against Claude Code 2.1.308 (docs/live-clear-compact.md): the compaction kept
a stated fact, `/clear` produced a new ID and the model no longer knew it, and resuming
the new ID in a new process stayed in the cleared conversation.

## Compaction notices

Claude's system status `compacting`, Codex's `contextCompaction` item start, and
Pi's manual `compaction_start` produce a started notice. No start is inferred
from sending `/compact` or accepting the command. Pi's own automatic compaction stays
disabled; BobLM sends `/compact` after a turn at the context threshold (ADR 0026).

The store assigns a compaction ID to each started notice and reuses it for the
confirmed boundary. The browser updates that row in place. A completion without
a preceding start remains a valid completed notice. A turn result or new process
init without a boundary ends the pending notice without claiming success.

History pages retain the first event as the row anchor and hydrate its latest
state at the page snapshot. Indexed compaction IDs prevent duplicate rows across
page boundaries and keep lifecycle lookup independent of total event count.
Notices use the chat font and size, with borders spanning the content width.
