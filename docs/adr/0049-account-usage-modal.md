# 0049: Account usage modal

A clock button in the sidebar header (shown when the browser has a token) opens an
Account usage modal. It replaces the sidebar Lock icon. The modal follows the
Questions modal pattern (ADR 0033): backdrop, `role="dialog"`, `aria-modal`, Escape
and Close dismiss it, Tab stays inside it, and focus returns to the clock button.
It is at most 36rem wide and fills the screen on phones. It has a fixed header, a scrolling
middle, and a fixed footer holding Lock. Its backdrop sits above the phone sidebar drawer,
which is where the clock lives.

The `accountUsage` command (owner-authenticated, empty request) returns one entry per
account: `label`, `kind` (`plan` or `api`), `provider`, and the stored five-hour,
weekly and credit readings from `account_usage` (ADR 0037). It carries no account
IDs, tokens, keys or paths. The browser requests it each time the modal opens and on
Try again; it does not poll. Each request carries a generation, and a reply for an
older generation (after a reopen or close) is discarded. A failure shows a plain
error with Try again; loading shows a status line.

## Accounts

Entries appear in this order: Codex plan, Claude plan A, Claude plan B (when
enabled), OpenAI API, Claude API A, Claude API B (when enabled). Enabled Claude
accounts come from the server's enabled-account configuration, the same one chat
creation and role routing use.

- Codex plan: the account ID of a Codex chat with the plan key (`codex-plan`) or no key.
- Claude plan A and B: the provider's own account ID for that plan key, else the ID the
  last chat on that key launched with (ADR 0047).
- OpenAI API and Claude API A and B: pay-per-token accounts with no allowance windows.

## Display

Plan accounts show a five-hour bar (only when reported) and a weekly bar as percent
remaining (100 minus used), with "Resets in 2d 4h", the local reset date and time, and
the age of the reading ("as of 3m ago"). Low remaining is the warning state, at the
same threshold as the composer meters. A window whose reset time has passed shows no
percentage and "This window has reset. Waiting for a new reading.", matching ADR 0037:
an expired allowance is never shown as a fresh one. Credits show only when the account
reports a positive balance, as a unit-less number with up to two fraction digits (ADR 0037
states no currency). API accounts read "Allowance is not reported for API
accounts." and an account with no readings reads "No readings yet."

## Lock

The sidebar no longer has a Lock icon. A quiet Lock button in the modal's fixed footer
starts the same confirmation flow as before; confirming disconnects this browser and
needs the server token to reconnect.
