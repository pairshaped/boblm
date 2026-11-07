# Archived chat deletion

Delete is available only for archived chats and requires confirmation in the browser. The session
runner serializes deletion with other commands, stops the agent, and removes BobLM messages,
events, tool names, schedules, credential requests, and upload records in one transaction.
Project files and the agent's own transcript remain outside this operation.

A deletion receipt retains only the chat ID so delayed creation requests cannot recreate it.
The global session-list stream retains a deletion event so other devices remove the chat.
Upload paths remain in a cleanup queue until their files are removed; deletion retries and
server startup retry that cleanup. A credential currently being saved must finish before its
chat can be deleted.
