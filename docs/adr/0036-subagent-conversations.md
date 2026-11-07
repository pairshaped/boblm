# 0036: Read subagent conversations in the chat drawer

The Agents tab lists the selected chat's subagents. Running entries have animated
activity dots in their agent's color. Selecting an entry opens its conversation
inside the drawer, under the existing tab bar. A header names the subagent and
provides a back arrow. Selecting Agents returns to the list; Questions remains
available. The conversation has no composer or steering controls.

The main chat and the drawer share text and tool rendering, message actions, and
the scroll controller. Each has its own scroll anchor and history cursor. The
drawer follows the newest output while at the bottom, holds a visible row while
the owner reads older output, and offers a return-to-latest button and the same
following lock indicator. Older pages load at the top, and the retained timeline
is bounded by the existing history window. The drawer uses its available width
with side padding.

The agent list is independent of the rendered message window. A derived index of
subagent lifecycle event sequences is maintained transactionally with the source
events and backfilled for existing chats. History snapshots include lean lifecycle
context containing identifiers, names, status and timestamps, without launch
prompts or tool output. The browser keeps the current roster separately from its
message rows. Per-agent revisions prevent an older page or snapshot from reviving
a completed agent. Reloading or changing chats restores the roster from durable
context, and live lifecycle events advance it even while the owner reads older
messages.

## BobLM subagent reports

A result event from a chat with a parent queues one row in the durable
`subagent_reports` outbox. The trigger records the child, the result event
sequence and the parent, so an old runner that has not restarted still queues
reports. The scheduler delivers a pending report only while the child is quiet:
its turn has ended, it has no background jobs, and nothing is queued, dispatching,
uncertain or held. A child with no provider process has no jobs. A running
provider must report an available, empty job inventory; unknown or unavailable
job state leaves the report pending. Pi has no background job tool, so its
inventory starts available and empty, as Claude's does. A restart leaves the row pending; it is not lost.

The report is rebuilt at delivery time from the child's latest assistant text, so
it reflects the final stored answer rather than a copy captured earlier. Oversized
text is shortened with a pointer to `inspect_peer`. The message also lists the
parent's open subagents with idle times and uses the parent-side reply trailer: no
reply is needed, reuse the subagent with `ping_peer`, or archive it when done.

Delivery uses the ordinary peer send path with the child as the recorded sender and
a UUID derived from the child session and result event sequence. It wakes an idle
parent and steers a busy one; a restart after the message was stored resolves to the
same message instead of adding another. A send error marks the row failed with the
plain-English cause and records a `subagent_report_failed` event in the child's
history. Failed and uncertain deliveries are never replayed automatically.

## BobLM child sessions

The Agents tab merges the selected root's BobLM children from the session list
with legacy native entries. A BobLM child opens its own durable chat history and
live event stream through the normal chat commands, not a provider transcript
poll. The header shows its role, model, effort and current status, with Stop and
Archive controls and no composer. Legacy native entries keep their provider
transcript reader and polling described below.

## Provider transcripts

For legacy native entries, `subagentHistory` is an authenticated, read-only HTTP command. The server resolves
the parent from BobLM's session metadata; the browser supplies a subagent key,
never a file path. Reading a transcript never launches, resumes or steers an agent.

Codex keys identify thread transcripts under its configured home. Session metadata
must establish an ancestor chain back to the selected chat's agent thread.
Inherited history before the subagent's recorded start ordinal is excluded when
that boundary is available.

Claude keys identify the Agent or Task tool call in the parent's transcript.
A published `agentId` resolves its child file. Before Claude publishes that ID,
a unique launch-prompt match within the parent's subagent directory can resolve
a running foreground agent. Ambiguous matches fail rather than selecting an
unrelated transcript. The child must identify the same parent session.

Canonical paths must remain inside the provider's session directory. A bounded
path index avoids rediscovering known transcripts on each refresh. Each read
scans a bounded file window, ignores incomplete final JSONL records, and converts
message text and tool calls/results into the existing browser event shape. System
instructions, reasoning records and binary media are not exposed by this adapter.
External formats can change; unsupported or unavailable transcripts produce a
plain-English error in the drawer.

The open conversation refreshes every two seconds as provider records are written.
This is message-level progress, not a promise of token-by-token output. Polling
stops when the drawer closes, the owner leaves Agents, the document is hidden,
or the selected parent changes. Older-page reads are snapshots. Request identities
reject late replies from previous selections. A latest-page read skips intermediate
history when returning to current output.

## Root summary

A root chat's list entry carries a server-computed summary of its subagents: open count,
unanswered questions, failed turns and failed report deliveries. The browser never
recomputes it; it refetches the chat list. A subagent's archive, reopen, delete or turn
change already appears as a list event. A question asked, answered, deferred or removed, and
a report that failed to arrive, are recorded only in the subagent's own event stream, so the
store appends a `subagents_changed` list event for the root in the same transaction as those
events. The summary is not pushed in the event: turn-end failures are written by runners that
may predate the event kind, so only a refetch sees every input.

The browser keeps at most one list request in flight. Further wants while it runs collapse
into one follow-up, sent when the reply arrives. Each request carries a number; a reply that
is not for the current request is ignored whole, so an older list cannot replace a newer one
or move the list stream's cursor backwards. A tab returning to the foreground abandons any
earlier request and asks again.

A consumed BobLM subagent report can qualify its parent for one idle nudge after
the parent ends that turn (ADR 0046). Pending reports and mid-turn children
suppress the nudge; subagent result events never create their own opportunities.
