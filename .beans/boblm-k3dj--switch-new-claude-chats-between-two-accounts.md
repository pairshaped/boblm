---
# boblm-k3dj
title: Run two Claude accounts with automatic handoff
status: todo
type: feature
priority: normal
created_at: 2026-10-31T17:11:17Z
updated_at: 2026-10-31T17:34:32Z
---

Support a second Claude plan (Ultra), so Claude work continues when one account's weekly allowance runs out.

## Decisions

- Owner decision 97 (2026-11-09): BobLM chooses the account automatically. No toggle, so ADR 0018 stands. New Claude chats use the account whose weekly reset comes first, until it has 2% of its weekly allowance left, then the other account.
- Both accounts can run at once: each has its own `CLAUDE_CONFIG_DIR`, logged in once. Claude children already receive `CLAUDE_CONFIG_DIR` (`crates/server/src/persistent.rs`).
- Direct handoff: when an account reaches the 2% cutoff, BobLM moves each of its chats to the other account at its next idle point. It copies the Claude transcript into the other config dir and resumes the same conversation ID there. New chats start on the other account. A chat is never moved back and forth, because the prompt cache does not carry across accounts and the first turn after a move pays for the full context.
- A quota or usage-limit error on a turn triggers the same handoff immediately, with a short note to the agent that its last turn was cut off.
- Reuse the Codex account-key pattern: per-session `account_key`, per-account usage readings (ADR 0037) and role routing readings, so the Sonnet implementer step and its reserve see both accounts.

## Open questions

- [ ] Verify Claude Code resumes a conversation from a transcript copied into another config dir. If it can't, the fallback is: chats finish on their account until its hard limit and then wait, while new chats use the other account.
- [ ] Decide how the R reserve for the Sonnet implementer step counts two accounts (combined remaining, or only the active account).

## Risks

- Reported lockouts of multi-account users seem tied to several accounts used from one IP or device. A ban could take out both accounts at once. The owner accepts this risk; keep the Pi driver (ADR 0026) viable as an exit path.
