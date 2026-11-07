# 0037: Context, allowances, and credits

The composer shows context used, five-hour allowance remaining, and weekly
allowance remaining when it has enough width. Meters are hidden on mobile.
The five-hour meter appears only when the provider reports that window. Pi
currently reports context only. Labels round to whole percentages. Context
fills with use; allowances drain toward zero. Context turns soft red at 90%
used, and allowances at 10% remaining. Missing or expired data is unavailable,
never an assumed fresh allowance.

Clicking Context opens a compaction confirmation. Confirming sends `/compact`
through the existing message delivery path, preserving the composer draft and
attachments. Clicking an allowance opens its reset date and time in local,
human-readable form. Five-hour resets show time only; Weekly includes the date.
These controls have no hover tooltips or tab stops.

Credits show as a remaining number whenever the provider reports a positive,
finite balance and available credits, regardless of the allowance windows.
Exhausted allowance bars remain visible at zero. Codex reports no starting credit total, so credits have no progress
bar. Missing balances and unlimited-credit accounts have no numeric display.

## Provider snapshots

Use the existing child process. Telemetry never starts an idle chat or creates
an inference turn. Claude's read-only `get_context_usage` explicitly requests
`detail: "summary"`, avoiding token-count API calls. Its `get_usage` request sets
`skip_behaviors: true`, avoiding a scan of local transcripts. Passive five-hour and seven-day
rate-limit observations refresh the corresponding samples. Control replies report
percent used; passive observations report a fraction.

Codex context uses the selected thread's latest `totalTokens` and reported model
window. Ignore child-thread context and cumulative totals. Quota samples come
from `account/rateLimits/read` and account notifications. Match the Codex bucket and window duration: 300 minutes for five-hour and
10,080 minutes for weekly. Either window can be primary or secondary. Credit
availability and balance come from that same bucket.
Pi's `get_session_stats.contextUsage` supplies its current-context estimate.

The readers poll at most once per minute for account usage. A child skips its
periodic poll when its account row was confirmed within that interval by any
child on the same account. Claude and Pi also refresh context after a turn
completes. Fixed, small metadata frames use a single nonblocking pipe write,
skipped when the input pipe is busy. These requests have no delivery waiter and
cannot stop the child for an unanswered control. Normal delivery acknowledgments
and control deadlines stay separate. Unsupported or malformed telemetry is
ignored. Provider details and primary sources are in
[the research note](../research/agent-usage-telemetry.md).

## Account identity

Allowance readings belong to the provider account, not to one chat. A child's
account ID is read from the provider's own local files when that child launches:
`oauthAccount.accountUuid` in the config directory's `.claude.json` for Claude
(`$CLAUDE_CONFIG_DIR/.claude.json`, or `~/.claude.json` without that variable),
and `tokens.account_id` in `$CODEX_HOME/auth.json` for Codex. These files are
undocumented, so missing fields and unreadable files are tolerated. Only the ID
field is used, no token value is logged, and a missing ID records nothing rather
than guessing an account. Pi and other pay-per-token providers have no account
row. The chat keeps the account it last launched with, so stopped chats still
show the same readings. A new runner also records the account at startup for an
existing chat, using the provider file in its own environment, so the mapping
comes back before the next child launch. A missing file leaves the existing
mapping alone rather than clearing it.

## Storage and delivery

`account_usage` holds one row per `(provider, account_id)` with five-hour,
weekly, and credit readings. Each allowance reading has its own reset time and
observed time; credits have their own observed time. The latest observation
wins: a reading is written only when its observation is newer than the stored
one, with no direction or delta heuristics. A bonus reset can therefore drop
usage inside a window, and an out-of-order observation cannot replace a newer
one. An unchanged observation does not rewrite the row while the stored
confirmation is younger than `ACCOUNT_USAGE_CONFIRM_REFRESH`, so write
contention stays low and the observation time remains useful for staleness.

New code reads context from the per-chat `session_usage` row and allowance and
credit values from the account row, never combined from another chat. Clear,
compaction, and model changes invalidate context and leave the account readings
untouched.

When an account row changes, the store publishes a fresh status to every open
chat on that account in the same transaction. Meter updates do not mark a chat
as working. An expired allowance becomes unavailable until the provider reports
the new window; never infer zero usage after a reset. Idle children can refresh
telemetry; a stopped child leaves its saved observation available with its
timestamp. The account row also supplies `list_peers`, `inspect_peer`,
`spawn_peer`, and the read-only `account_usage` peer tool, which reports every
account with its reset and observation times, expiry, and elapsed window share.

## Per-chat usage

`session_usage` keeps only the context meter (`context`, `context_at`). Migration 039
removed the per-chat weekly, five-hour and credits columns and the trigger that
filled them; `remember_usage` records context only. Allowances come from
`account_usage` alone. The per-chat usage snapshot carries context only, and the
browser draws allowance meters from the account row only. Token and allowance
history reporting is described in ADR 0042.

The browser schedules the earliest future allowance reset in its existing fixed
subscription slot. Expired five-hour samples retain evidence that the window is
supported, but supply no remaining percentage until a fresh sample arrives.
