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
Deleting a question removes it from the drawer, pending count and answer outbox.
Retired IDs remain reserved, so old create or update retries cannot restore deleted
prompts. Pending push notices for retired prompts are suppressed. Deletion shares
a lock with answer handoff: it either suppresses the pending answer or follows its
handoff. Messages already handed to the runner remain in chat history.

The Questions tab presents the viewed chat's unanswered prompts as a durable FIFO queue. It shows
the current question and how many questions are waiting after it. The waiting line is hidden when
there are none after the current question. There is no cap on how many unanswered questions can
accumulate. Each response fetches the current prompt and recent decisions rather than downloading the
whole backlog. Queue positions are allocated in the same transaction as each question, so questions
created in the same millisecond still have a stable order. The queue survives reloads and restarts.

Suggested answers are radio buttons, followed by Discuss later and then Type a different answer,
which opens a text field. Clicking a suggested answer or Discuss later anywhere in its label submits
that decision immediately. Only a custom text answer needs Submit. A question without suggestions opens
the text field directly. Answering advances to the next prompt.

Questions and Agents share one right-hand drawer with tabs. One chat-header toggle opens and closes
it, and tab changes preserve question drafts. It starts closed on a normal page load. The Questions tab is selected on page load; question
notification links open it directly. Tabs show unanswered question and active agent counts. Both
desktop side drawers are 320px wide. On phones the right drawer fills the screen, hiding the chat header. Its tabs and drawer toggle
share a single header row. Desktop uses the toggle in the chat header.

Discuss later means: "I'm not sure. Let's discuss once there are no more open question prompts."
It removes that question from the unanswered prompt queue without marking it answered. Its state is
`discuss_later`, and its answer stays null. Deferred questions are kept out of the drawer.
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
rules. Deleting a chat deletes its questions. Question changes publish a chat event; snapshots load
independently of the bounded message-history window and refresh after its cursor is established.
Browser request generations discard stale snapshots. Per-question text, choice and answer limits,
and the number of recent decisions returned, live in tuning.
