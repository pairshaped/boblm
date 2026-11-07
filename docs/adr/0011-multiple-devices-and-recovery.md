# 0011: Multiple devices and recovery

The owner may have a phone and a browser open on the same session at the same
time. Both see the same state, either can send, and neither can corrupt it.

The server is the only writer. Clients send commands and never send state, so
there are no version columns and no optimistic update rejection. Every
command for a session is serialized by that session's runner task; two devices
sending or steering at once resolve in arrival order and both messages are kept.
Rename, archive and similar actions are last-write-wins.

Every event carries a per-session monotonic sequence number, and the session
list has its own global sequence (ADR 0005). A client resumes from the last
sequence it received and loses nothing, whichever device it is.

Every client message carries a client-generated UUID, and the server treats a
repeat of that UUID as the same message. A phone retrying after a lost reply
cannot send twice: an identical retry returns the existing message, and the
same UUID with different content is rejected.

Composer drafts live in the browser, per device. Devices do not share drafts.

Sessions are recoverable. The store keeps the agent's own session id. A web-only restart leaves supervised runners and providers running (ADR 0040).
After a runner restart or archive, the next owner message resumes the agent
session with its context. Runner restart converts unresolved
`dispatching` input to `uncertain`, which is never replayed. Queued input stays
queued until the owner sends or resumes. A due scheduled prompt can start an
idle chat under the hold rules in ADR 0004 and ADR 0022. History, the resume id, model and effort survive archive.
