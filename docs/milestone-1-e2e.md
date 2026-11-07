# Milestone 1 browser verification

The real BobLM serve binary and browser passed create, mid-turn steering,
server restart with context resume, and active archive/reopen on 2026-10-21.
The final workspace check passes 45 Rust tests and 16 TypeScript tests and
builds the browser assets.

## Environment

The server bound 127.0.0.1:39382 and served web/dist. Claude Code 2.1.306 used
an isolated configuration, temporary directory, working directory and copied
credentials under target/e2e. A local PATH wrapper supplied empty setting
sources and an empty strict MCP configuration. No owner hooks or MCP servers
were loaded. The browser used the installed Playwright headless Chromium
through the named playwright-cli session boblm-e2e-j4c8.

The server binary was linked with cargo build -j4 -p boblm-server --bin boblm
because this verification needs a runnable executable. Browser assets came
from npm run --prefix web build. The server ran as boblm serve --config
target/e2e/config.json, with the isolated environment supplied by the harness.
The scripts and disposable SQLite database remain under target/e2e and
target/browser. The copied credentials were removed after verification.

## Results

1. Created a session through the browser and selected Sonnet with low effort.
   The first prompt supplied a codeword for later context verification.
2. Started a foreground Bash call that wrote a readiness marker, slept for
   20 seconds, and completed. The original prompt required a separate later
   Bash call to write WRONG. After readiness, sent an ordinary browser message
   redirecting the agent to write STEERED instead. Claude reported the original
   turn cancelled, started the steer UUID, wrote STEERED and replied STEERED.
   The separate WRONG action never ran. Both messages were persisted as started.
3. Stopped the server after four persisted messages. Restarted with the same
   database. The browser restored history and showed Session not running.
   No new owner input was executed on startup. Asked for the original codeword
   without including its value. Claude returned the codeword. The saved Claude
   session ID, Sonnet model and low effort were unchanged.
4. Started another foreground Bash call that would write WRONG after a
   15-second sleep. Archived the session after its readiness marker appeared.
   Archive interrupted the call and stopped the process. Reopened and checked
   that the session remained stopped. A subsequent browser message recalled
   the original codeword in the same Claude session. Checked after the original
   sleep deadline: WRONG was absent. No message remained dispatching.

## Steering boundary

An initial probe put WRONG inside the already running shell command. That
probe wrote WRONG even though Claude later replied STEERED. This is consistent
with the reference: priority now redirects at the next tool boundary, and does
not cancel the current tool call. The corrected probe makes WRONG a separate
next action and passes. Stop and archive use the interrupt control to cancel a
running tool. No implicit interrupt was added to ordinary message steering.

## Commands and artifacts

The browser runs used playwright-cli run-code with these scripts:

- target/browser/e2e-start.js and e2e-steer.js: initial boundary probe.
- target/browser/e2e-boundary-start.js and e2e-boundary-steer.js: passing steer.
- target/browser/e2e-resume.js: restart, history and remembered-context check.
- target/browser/e2e-archive-start.js and e2e-archive-reopen.js: archive interrupt
  and reopen without execution, followed by context resume.

Workspace output is target/web-check.log. The earlier direct model/effort/Bash
interrupt check is target/model-stop-live.log. Browser API requests returned
200 during the successful runs. Network errors during the intentional server
restart were expected; event subscriptions reconnected with their cursors.
The browser session, server and owned Claude processes were stopped afterward.

Desktop and phone layout screenshots are target/browser/desktop.png and
phone.png. Owner visual review and the manual Safari checklist remain pending;
Linux Chromium does not verify Safari caret rendering or IME behaviour.
