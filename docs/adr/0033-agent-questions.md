# 0033: Asynchronous agent questions

The `ask` MCP tool records a question in the caller's own chat and returns immediately. Agents supply
a fresh question UUID, text, and optional suggested answers. An identical retry returns the existing
question. Reusing an ID for different text, choices or a different chat fails. Credentials belong in
`request_sops_secret`; question text and answers are ordinary persisted chat data.

Agents can revise an unanswered prompt with `update_question` or remove a resolved
prompt with `delete_question`. Both tools are restricted to the caller's chat.
An update supplies the full revised text and choices plus a fresh `replacementId`.
The revised prompt keeps the original queue position and uses that new ID. Answers
submitted from an old screen are rejected because the original ID is retired.
Answered and deferred prompts cannot be revised; agents can delete them and ask a
new question when another decision is needed.

Identical update retries reuse the replacement ID and do not change the queue.
Deleting a question removes it from the dialog, pending count and answer outbox.
Retired IDs remain reserved, so old create or update retries cannot restore deleted
prompts. Pending push notices for retired prompts are suppressed. Deletion shares
a lock with answer handoff: it either suppresses the pending answer or follows its
handoff. Messages already handed to the runner remain in chat history.

The shared Questions modal presents unanswered prompts from all chats and subagents as a durable
FIFO queue, ordered by creation time then question ID. Archived chats are included; deleted chats
and retired questions are excluded. Each prompt shows its originating chat, with a subagent label
and parent chat name when available. It shows the oldest pending question and how many are waiting.
Recent answered and deferred decisions are bounded by the configured list limit.

Suggested answers are radio buttons, followed by Discuss later and then Type a different answer,
which opens a text field. Clicking a suggested answer or Discuss later anywhere in its label submits
that decision immediately. Only a custom text answer needs Submit. A question without suggestions opens
the text field directly. Answering advances to the next prompt.

A dedicated question-mark button in the chat header, immediately before the drawer toggle, opens
the modal and shows the global pending count. The right-hand drawer has Agents and Jobs tabs.
The modal starts closed on normal loads. Question notification links open it and focus the question.
Escape and Close dismiss it. Drafts persist across reloads and chat changes. Answers and discussion
requests use the originating question's chat ID, independently of the viewed chat.

Discuss later means: "I'm not sure. Let's discuss once there are no more open question prompts."
It removes that question from the unanswered prompt queue without marking it answered. Its state is
`discuss_later`, and its answer stays null. Deferred questions are kept out of the dialog.
The discussion request is held durably until this chat has no unanswered prompts. Normal answers are
handed off before discussion requests. The agent then receives the original question and the owner's
request to discuss it. A discussion request is never fabricated as an answer.

The first answer or discussion decision wins across devices. Each decision and its pending delivery
are saved in one SQLite transaction. The runner receives a stable message UUID, so a lost reply or
restart can retry the handoff without replaying uncertain or held input. A `started` or `completed`
acknowledgment still proves agent consumption. The browser reports saved decisions, without claiming
the agent has read them. Further conversation about a deferred question happens in the chat.

The scheduler drains pending decisions after restarts and during normal operation. Answering or
requesting discussion is an owner action and can reopen an archived chat under the normal owner-message
rules. Deleting a chat deletes its questions. Question changes publish both a chat event and a durable questions_changed list event. Global
snapshots load after the list baseline and refresh on list events and reconnects.
Browser request generations discard stale snapshots. Per-question text, choice and answer limits,
and the number of recent decisions returned, live in tuning.
