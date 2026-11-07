# Narrow phone layout check

Checked on 2026-10-23 with the isolated fake-driver browser fixture and the named
Playwright CLI session `boblm-xvy7`. The fixture used a long project name and path,
and a Pi chat with a long model identifier. No owner data or real model calls were
used.

Before the fix, the 320-pixel viewport had a sidebar with 357 pixels of content,
and the New chat dialog had 398 pixels of content within 286 available pixels.
The browser assertion failed on these measurements. Inputs and selects also used
fonts smaller than 16 pixels.

The fix allows grid tracks and controls to shrink, wraps long names and paths,
and sets mobile control text through `--font-size-mobile-control` in theme.css.
The repeated Chromium check at 320, 375 and 390 pixels found no horizontal overflow
in the sidebar or New chat dialog. The folder picker and login form fit at 320 and
375 pixels. The chat and composer fit at 375 pixels. Desktop testing at 1280 pixels
kept the sidebar's existing 240-pixel column, with 239 pixels of content inside its
border and no horizontal overflow.

Mobile controls compute to 16 pixels. Safari's automatic zoom and the physical
phone's keyboard behavior still need confirmation on the owner's iPhone; these
Chromium checks do not verify Safari.

Final `bin/check` exited 0 with `check passed`, including all 86 web tests.
The browser session and fake server were stopped; `playwright-cli list` showed no
open browsers and `pgrep` found no remaining fixture process. Disposable data and
logs were removed.
