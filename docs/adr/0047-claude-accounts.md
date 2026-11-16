# 0047: Claude accounts and conversation handoff

BobLM pins every new Claude chat to an account key. `claude-plan-a` is the
existing default Claude login, including an inherited `CLAUDE_CONFIG_DIR`.
A legacy Claude chat with no key means plan A. `claude-plan-b` has its own
config directory beside the database. `CLAUDE_ACCOUNT_HOMES` maps those homes
and the isolated `claude-api-a` and `claude-api-b` homes. Plan A preserves
Claude's default lookup when `CLAUDE_CONFIG_DIR` is unset, so its account file
remains `~/.claude.json` and its transcripts remain under `~/.claude/projects`.

`CLAUDE_PLAN_ACCOUNTS` and `CLAUDE_API_ACCOUNTS` enable accounts. Each starts
with A only. Enable plan B after logging into its home once; enable API B after
saving `ANTHROPIC_API_KEY_B` in the configured SOPS file. The login steps are in
[Running BobLM](../running.md). There is no account picker or settings screen.

## New chats

Owner-created chats, explicit Claude peers and Claude role steps choose their
own account. Subagents never inherit a parent's account. Among enabled plan
accounts with more than `CLAUDE_ACCOUNT_CUTOFF_PERCENT` (2%) weekly allowance
left, choose the one whose weekly reset comes first. Ties retain configuration
order; unknown reset times sort last. This drains one account before using the
next instead of spreading new chats across them.

Owner-created chats and explicit-model peers are always allowed. With one enabled
plan they use that account even at the cutoff, preserving existing behavior.
With multiple plans they prefer the available account described above; if every
account is at the cutoff or marked exhausted, they still use the one whose weekly
reset comes first. Only an empty enabled list prevents creation. Role spawns keep
the cutoff in both configurations and fall through or refuse when none has room.

Usage comes from each home's own Claude account UUID and that UUID's
`account_usage` row (ADR 0037). A stored chat mapping supplies the identity if
its account file is unavailable. API identities never supply plan readings.
Missing readings and expired reset windows count as available for routing,
consistent with ADR 0043; the meters continue to report unavailable data.
Expired samples do not become invented zero-usage observations.

Implementers compare the selected Claude account with the Codex plan and use the
available step with the earliest known future weekly reset (ADR 0043). Ties and
unknown or expired readings retain Codex-first table order.

The Sonnet implementer step has no reserve. It uses only the 2% cutoff, with one
plan account or several.
Each plan and API account has its own open-chat and reviewer caps (ADR 0043).
Role spawns filter full plan accounts before applying the existing reset-first
selection and allowance cutoff. Explicit model spawns filter full accounts and
retain their allowance fallback when every plan is exhausted. Owner account
selection remains unrestricted.
Haiku API reviewers choose the first enabled API account with room and credit
that has no active exhaustion mark, in configuration order. The reviewer
spawn is refused when no enabled API account is available. Unknown credit balances are allowed.
Quota or billing failures mark the individual API account until the configured
monthly credit grant day, the 27th UTC (`ANTHROPIC_API_CREDIT_RESET_DAY`). This is
account A's credit grant day, independent of billing. B shares it until the owner
links B and confirms its grant day. An older global Haiku exhaustion mark applies to A only.

## Handoff

The serialized runner checks for handoff at actor boundaries and at the usage
poll interval. When its current plan is at the cutoff or has reported a usage
limit, it can move to another enabled account below the cutoff. It requires no
active turn, compaction, dispatch, pending acknowledgment, outstanding tool, or
background job. A running child needs an available, empty job inventory.
Owner Stop, archive and held queues retain their existing behavior.

The actor stops the idle child, copies the conversation's `projects/<project>/
<conversation-id>.jsonl` and companion `<conversation-id>/` directories to the
same paths in the destination home, then atomically changes the stored account
key and provider identity. It copies all matching project folders so a changed
working directory does not lose the transcript. Credentials, unrelated chats,
and project memory are not copied. Files are private and replaced atomically.
The conversation ID stays the same and the next child receives it through
`--resume`. A failed copy keeps the source key and holds the chat with a repair
message; it never starts a fresh conversation in its place.

A durable per-chat exclusion prevents returning to the source account before
its weekly reset, even if a bonus changes that account's reported usage. The
prompt cache does not transfer between accounts, so the first resumed turn
pays for the context again. If a
reset is unknown, use one weekly window from the handoff. The exclusion survives
runner replacement. A provider usage-limit failure also marks that account
exhausted and records a pending continuation. The handoff transaction queues
one automated note saying the last turn was cut off and asking the agent to
continue. It never replays an uncertain message. With no eligible destination,
the chat stays on its account and waits as before. API chats do not hand off.

The migration adds tables only. Existing runners retain their loaded behavior
until replaced safely under ADR 0040. Enabling B requires rebuilding and safely
replacing those runners as well as the web server.

The owner accepts the risk that using multiple plan accounts from one host could
result in both accounts being restricted. Pi remains available as another provider.

## Manual move

The owner can move an idle Claude plan chat to the other plan account with the
authenticated command `moveClaudeAccount` `{sessionId, accountKey}`, which replies
`{sessionId, from, to, status}`. `from` and `to` are plain labels ("Claude plan B");
`status` is `moved`, or `already_moved` when an earlier move already put the chat on
that account and left its exclusion in place. `bin/move-chat-account <id or prefix>
<a|b> [--wait SECONDS] [--no-compact] [--dry-run]` wraps it. The server performs the
move, so a runner started from an older executable needs no new code and no
replacement.

The command accepts only an enabled Claude plan account for a root Claude chat on a
plan account. It refuses, with a sentence that says what was refused and that nothing
changed, a Codex or Pi chat, a subagent, an API chat, an API or unknown destination, a
destination that is not enabled, a destination with no signed-in account in its home,
and a destination equal to the current account (unless that is the finished earlier
move above). It refuses a chat that is not idle with a sentence that names what is running and
ends "Try again when it is idle." Idle means: not archived, not stopped by the owner,
no active or stopping turn, nothing queued, dispatching or uncertain (live status and
stored messages agree), no open subagent, an available and empty job inventory, and a
newest activity event that is a turn result at least `RUNNER_QUIET_PERIOD` old
(the runner restart's own test, ADR 0040). A chat that is idle except for that quiet
period gets error code `settling`, which callers may retry at once; other idle
refusals carry `not_idle`. One move per chat runs at a time.

The server reuses everything the automatic handoff uses. It stops the idle child
through the owner's archive path, verifies the child is gone, copies the transcript with
`copy_transcript`, and commits the switch through one store transaction that shares
`handoff_claude_account`'s writes: account key, provider identity, the durable exclusion
from returning to the source before its weekly reset, and the `claude_account_changed`
event. The manual transaction also requires that the chat is still archived by the move,
still on the source account and without accepted input, so a message that arrives
mid-move wins and the move is abandoned. Reopen then leaves the chat for its next
message to start a child with `--resume` on the same conversation ID and the
destination account. A failed stop, copy or switch reopens the chat on the source
account with a plain error; nothing starts a fresh conversation in its place. Archive
cascades to subagents, which is why open subagents refuse the move.

The prompt cache does not transfer, so the first turn after a move pays for the whole
context again, about the chat's `usage.context.usedTokens` at the move. The script
therefore sends the existing `/compact` command first when context is at or above
`CLAUDE_MOVE_COMPACT_PERCENT` (90) unless `--no-compact` is given, waits for the
compaction to finish and for the chat to settle, then moves. With `--dry-run` it
prints the plan and changes nothing. Exit codes: 0 moved, 1 refused or not idle, 2
usage or error.

## Transcript-copy evidence

On 2026-11-09, Claude Code 2.1.319 ran an isolated Haiku conversation under
`target/claude-account-live/source` used a copy of the existing plan credentials.
It was told: "The test observatory is named Cobalt Finch 731." Only the
conversation transcript and companion directory were copied to the named
`claude-accounts` dev instance's `data/claude-api-a` home. The destination had
no subscription credentials. Its Haiku process received API A's SOPS key only
through its environment.

`--resume 8c2f436d-4403-4100-b1e9-155f997e4715` under that API home returned
"Cobalt Finch 731" and the same conversation ID. Both turns disabled tools and
used a short system prompt. Reported costs were $0.000131 and $0.000230075,
totaling $0.000361075. An earlier isolated check also passed; all four turns
reported $0.003139995 combined. Copied credential and config files were removed
afterward. This verifies direct handoff, so the Bean's fallback of leaving every
chat on its original account is unnecessary.

The dev-instance smoke created and listed a Claude chat pinned to plan A without
starting inference, then loaded it in the authenticated browser with its composer
and no console errors. After the owner-creation policy clarification, a rebuilt
server also created a plan A chat with an active account exhaustion mark. The
browser sessions and dev instance were stopped after verification.
