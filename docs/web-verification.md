# Browser verification

The Web checks use Hypertea's built public program and testing exports.
The dependency is the public pairshaped/hypertea repository pinned by commit in
web/package.json. Its prepare script builds dist on install, and esbuild bundles
those files.

Automated DOM tests assert exact draft text, backward selection and focus while
20 events append. Enter sends, Shift+Enter and IME composition do not send,
and sending sets readOnly without disabling the textarea. Other tests cover
pending selection, delivery state rendering and explicit UUID-preserving retry.

The named playwright-cli session boblm-web-q0qv used the installed headless
Chromium executable and an isolated fake-driver server. It verified streaming
text/selection/focus preservation, Enter, Shift+Enter, reading-position retention,
New messages returning to the bottom, visible pending selection, and no
horizontal overflow at 390 by 844 pixels. The clean rerun had zero browser
console errors and all HTTP commands and SSE connections returned 200.
The browser session and fake server were closed after the checks.

Desktop and phone review images are target/browser/desktop.png and
phone.png. The executable smoke script is target/browser/web-smoke.js.
Aesthetic acceptance awaits owner review of those images. Safari caret and
IME behaviour await the manual docs/safari-checklist.md run on Safari.
