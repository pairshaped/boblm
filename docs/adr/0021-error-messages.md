# 0021: Error messages

Every message a person or an agent can read says what happened, why when that is known, and what
to do next, in plain English. The owner is the only user, so the technical cause belongs in the
message; a stack trace, a debug dump or a bare code does not.

A message names the thing involved (the chat, the field, the file, the model), quotes the limit or
the value that was wrong, and ends with the next step when there is one. "invalid session id" is
replaced by "sessionId must be 1 to 128 characters and not blank; got \"\"". Client-supplied values
are quoted but cut at 40 characters, so a huge input cannot flood a reply.

Faults on the server say what the server was doing and why it failed ("The server could not write
the upload to disk (is the disk full?): No space left on device"), state that the input was not at
fault, and point to the server log, which receives the same text. They carry no stack trace or
debug formatting. The machine-readable `code` in the error envelope stays stable for programs
(`invalid_request`, `not_found`, `message_conflict`, `unavailable`, `internal`, `unauthorized`).
`internal` and `unavailable` can follow work that was already done, so clients treat them as an
unknown outcome, not a refusal (ADR 0020);
the `message` is always the sentence for people. A new field in the envelope for extra detail is
allowed, but the browser never shows it.

An error whose outcome is unknown says so and says what is safe: a lost or unreadable reply states
the cause (could not reach the server, an HTTP status with a body that is not a BobLM reply, a
reply in the wrong shape, a reply over the size limit) and, for a send, that Retry reuses the same
message ID and cannot deliver twice (ADR 0020). A browser decoder failure names the command and the
first field that is wrong. A broken live stream is shown once, with its cause, in the chat, and
clears itself when the stream recovers; it is not retried silently.

Messages meant for Claude (peer tool errors, driver events recorded for the chat) follow the same
rule and name the tool or action that failed, the identifier that was not found, and the call that
would list the valid ones.

`bin/check-messages` fails the check on a short list of vague phrases ("operation failed",
"unknown error", "something went wrong"). Tests assert the wording of the messages people meet
most: request validation, missing or archived chats, conflicts, transport failures, a missing
Claude binary and control timeouts.

Codex diagnostics go to the server's stderr, collected by the service journal, with the child
process ID and Codex thread ID. The driver drains stderr alongside protocol output, caps each
line with `MAX_AGENT_DIAGNOSTIC_BYTES`, and redacts the peer-tool capability. Oversized lines
are omitted with a diagnostic; malformed protocol lines report their position without copying
the payload. Process exits and pipe errors are logged even when no turn result arrives.

Codex `error` notifications appear in the chat and the journal, including whether Codex will
retry. An error notification proves neither consumption nor completion and does not extend
the delivery deadline. Runner logs identify the BobLM session; delivery timeouts also name the
agent thread and unacknowledged message IDs. Send failures retain their cause in a durable
`driver_error` event and the journal. Unconfirmed input stays held for inspection.
