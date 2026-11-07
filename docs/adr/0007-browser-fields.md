# 0007: Browser fields and subscriptions

The composer is a plain textarea with Hypertea's uncontrolled value property.
Its declared value stays constant while typing and while events append.
Input events save a per-device draft in browser localStorage without assigning
the textarea value. Successful sends explicitly clear the matching session's
field and preserve focus. Navigation restores that session's local draft.
A response for a session that is no longer selected cannot clear the new field.
While a send is in flight the textarea is readOnly, never disabled.

Enter sends unless Shift is pressed or an IME composition is active. Shift+Enter
keeps the native newline behaviour. Lowercase onkeydown and onmousedown bind
direct DOM events through Hypertea's public h function. Send's mousedown keeps
focus in the composer. Rejected delivery offers Retry; HTTP uncertainty offers
an explicit Retry send using the original UUID. Neither path retries itself.

The message-list scroll observer remembers whether the reader is within 80 px
of the bottom. Appended content follows only in that state. Otherwise it keeps
the reading position and shows New messages. Selecting that control scrolls to
the bottom and resumes following. List observers and SSE subscriptions dispose
on navigation or logout. Cursors stay out of subscription keys.

Hypertea is a git dependency on the public pairshaped/hypertea repository,
pinned by commit in web/package.json. npm runs its prepare script on install,
which builds dist. Its public exports resolve to dist/index.js, dist/program.js
and dist/testing.js with their type declarations. Esbuild bundles these built
exports into web/dist/app.js. Updating Hypertea means reviewing the upstream
diff and bumping the pinned commit. Tests use Hypertea's public testing mount helper with
jsdom. The repository's npm cache is under target/npm-cache.

Browser checks cover the real HTTP/SSE adapter with an isolated fake-driver
server, exact text/selection/focus preservation, Enter and Shift+Enter,
reading-position retention, bottom following and phone overflow. Safari caret
and IME behaviour still require the owner's Safari checklist. Desktop and
phone screenshots are review artifacts under target/browser.

Initial connection validates the entered token with a dedicated request before saving it or starting
live subscriptions. Each attempt has its own callback identity, and stale replies cannot finish a
later attempt. Successful validation saves the token before follow-up requests run. The connection
check and response decoding are bounded by BROWSER_CONNECT_TIMEOUT_MS; a timeout leaves the modal
open for an explicit retry.

Subscriptions keep fixed positions, including an empty slot for the optional
weekly-usage reset timer. Hiding the page, loading history, and locking the app
cannot shift the browser listeners into different slots. A visibility handler
created while the page is hidden queues its first update until subscription
installation finishes; cleanup cancels that queued update.
