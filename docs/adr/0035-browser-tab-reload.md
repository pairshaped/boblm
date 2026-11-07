# 0035: Targeted browser-tab reload

The caller-bound `reload_ui` tool targets one focused, visible page viewing the caller's chat. A page
gets a fresh identity for each chat subscription and advertises reload support through its presence
heartbeat. The server suppresses reloads when it has no supported target or more than one candidate.
Other tabs and devices keep their documents open.

Reload requests and their receipts stay in server memory. The target's next heartbeat consumes the
request before the reply is sent, so a lost reply cannot cause a later repeat. Requests expire after
`BROWSER_RELOAD_TTL`; a blur, chat change, new page identity or server restart discards delivery.
The browser changes its identity after a failed presence request. Receipts deduplicate explicit
retries until server restart, with a fixed receipt limit. Tool acceptance means queued, and does not
prove the browser consumed the reply or refreshed.

Before reloading, the browser saves chat drafts, custom question answers, completed attachments,
in-flight message IDs and the viewed chat. It keeps the document open if an attachment is unfinished
or local storage rejects a write. This control uses the live presence path so loading chat history
cannot replay a document reload. It does not restart the server or send chat input to an agent.
