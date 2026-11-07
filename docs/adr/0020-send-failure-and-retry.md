# 0020: Send failure and retry

A send ends in one of three ways, and the composer treats each differently. In every
case the draft text and the chosen attachments stay in the composer, and the error
appears beside the composer, for that session only, with a Retry button.

Accepted: the reply says so. The draft and its attachments are cleared.

Refused: the server answered with a typed error that means it did not take the message
(invalid input, an unknown or unavailable attachment, a closed session). Nothing was
stored, so Retry is an ordinary send with a new message UUID, using the current draft.

Uncertain: no valid reply arrived, or the server answered `internal` or `unavailable`. Both can
follow a saved message, for example when reading the status afterwards fails or the chat's task
stops, so they are held like a lost reply. The message UUID, text and attachment list are held,
and Retry resends exactly that under the same UUID, which the server treats as the same message
(ADR 0011). Nothing is ever replayed automatically (ADR 0005). While a send is held, no other
send starts in that chat: the composer asks the owner to Retry it or Discard it first, because
the held message may still arrive and a different send must not overwrite its record. Discard
forgets the held ID and keeps the draft.

A send is written to the browser's local storage, with the drafts, before its request leaves,
and stays there until the reply settles it. A reload while a request is in flight, or while a send
is held, therefore never loses its UUID. After a reload, or after locking the app, the composer says
the earlier send's outcome is unknown and offers the same Retry and Discard. Removing an attached file that a held send names drops the held send,
because a send must not change under its UUID; the composer says it may still have arrived.

A delivery that Claude later rejects is a different failure, shown on the message with its
own Retry (ADR 0004). Nothing here tracks it.

A message the agent never confirmed (delivery state `uncertain`) is listed in a banner above the
composer, with the plain statement that it may or may not have reached the agent and is never resent
on its own. Each entry has Send again, which puts its text in the composer (after any text already
typed) for the owner to review and send as a new message, and Dismiss, which hides the entry. Both
hide the entry. The hidden entries are remembered in the browser's local storage by message UUID and
change nothing on the server; the message stays in history as unconfirmed. Attachments are not carried
over by Send again.
