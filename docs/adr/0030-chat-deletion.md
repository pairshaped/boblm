# Archived chat deletion

Delete is available only for archived chats and requires confirmation in the browser. The session
runner serializes deletion with other commands, stops the agent, and removes BobLM messages,
events, tool names, schedules, credential requests, and upload records in one transaction.
Project files and the agent's own transcript remain outside this operation.

A deletion receipt retains the chat ID so delayed creation requests cannot recreate it.
Agent deletions also retain the deleting session ID, including on cascaded subagents.
The global session-list stream retains a deletion event so other devices remove the chat.
Upload paths remain in a cleanup queue until their files are removed; deletion retries and
server startup retry that cleanup. A credential currently being saved must finish before its
chat can be deleted.

Agents use `delete_chat(sessionIdPrefix, requestId)`. The server resolves full IDs
and unique prefixes within the caller's project. Missing targets and targets in
other projects receive the same refusal. The target must be archived, must not
be the caller, and must have no unanswered owner question. Unanswered means not
retired, without an answer, and not set aside for discussion, as for idle nudges.
A root with any unarchived subagent is refused. The same question and self-deletion
guards cover every subagent in a cascade.

Agent deletion uses the shared deletion coordinator, receipt writer and upload
cleanup. It first stops and pauses child runners, then the root's transaction
rechecks the guards and removes the root and all its archived subagents. The
coordinator tells the paused child runners to exit after commit, or releases them
if deletion is refused. A paused child checks its deletion receipt on the normal
usage poll, so it exits after commit even if the web process crashes. Repeating
preparation is harmless, and an owner reopen releases an abandoned preparation.
Owner deletion keeps its existing command sequence and permissions, including
deletion with unanswered questions.

The guarded deletion and child preparation actions need the current runner code.
Older persistent runners keep supporting owner deletion. Restart those runners
when their work is safe to interrupt before using agent deletion (ADR 0040).
The migration only adds a nullable receipt column and a new table, so older
runners can keep using their existing SQL.

An agent supplies a fresh requestId UUID. Its receipt records the bound caller,
original prefix and resolved chat ID in the deletion transaction. An identical
retry returns the same receipt, even after a restart or when the prefix later
matches a different chat. Reuse by another caller or with another prefix is
refused. HTTP disconnection does not cancel deletion or upload cleanup.

Agents should delete only finished chats whose history is no longer needed.
Deletion permanently removes BobLM's history, uploads and stored transcripts.
The provider's own transcript files remain outside this operation.
