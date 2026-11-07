# Wake idle Codex parents on subagent completion

Codex delivers subagent final answers at tool boundaries while the parent is
mid-turn. Between turns, the persistent session runner observes Codex's parent rollout for completed
`SubAgentActivity` records and native `agent_message` final-answer receipts.
The scheduler checks these records without starting an agent or making an
inference request. The observer handles completions recorded after it starts;
it does not replay historical completions when the runner starts. A web restart
leaves this observer running, so completions during that restart remain visible.

A completion identifies a child thread, task path and child turn. The child
transcript must name this parent, and its matching `task_complete` record
supplies the final message. Missing child output is checked again on later
passes. A native final-answer receipt removes the pending completion. Mid-turn
parents use Codex's native delivery; if the turn ends without that receipt,
the completion remains eligible for the idle fallback.

Runner accepts an idle wake only between turns, with no consumption
acknowledgments outstanding. It leaves stopped, held, archived and disconnected
chats alone. The message uses the ordinary peer queue, delivery acknowledgments
and durable UUID receipt.

The wake contains a short pointer naming the completed task, without its result
body. It asks the parent to run `true` once so Codex can drain its queued native
final answer at that tool boundary. Agent-inventory tools can return the full
result too, so the pointer avoids those tools. The app-server exposes no mailbox inspection
or clearing operation, so BobLM checks actual delivery in the parent transcript. If that turn ends without a native
final-answer receipt, the next idle pass queues the full body as a fallback.
A native receipt can arrive after the model's final answer. Only subsequent
model output proves that the result was available for consumption. If the
parent becomes idle with an unconsumed native receipt, BobLM sends a short
pointer to the result already in its context. This never repeats the body.

Each stage has its own stable UUID derived from the parent thread, completion ID
or native receipt ID, and stage. Repeated events cannot dispatch a stage twice.
Large fallback bodies are shortened to fit the message limit, with a pointer to the transcript.

Accepted wake receipts stay in FIFO order until native delivery, so an older
native receipt cannot suppress a later completion from the same child. A child
that finishes mid-turn uses native delivery first. If the turn ends without a
receipt, it follows the same pointer and fallback sequence.

Parent reads advance through bounded chunks and retain incomplete final records
for the next pass. Oversized records are discarded through their terminating
newline. Dispatch waits until the reader has caught up, so a later native
receipt in the same backlog can suppress the fallback. The provider transcript
is external input; unknown records are ignored.

Injected guidance tells agents that subagent results arrive later as messages.
Agents can continue other work or end their turn after spawning background work.
They do not need to poll for completion.
