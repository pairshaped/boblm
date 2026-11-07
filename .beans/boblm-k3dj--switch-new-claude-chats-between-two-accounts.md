---
# boblm-k3dj
title: Switch new Claude chats between two accounts
status: draft
type: feature
priority: normal
created_at: 2026-10-31T17:11:17Z
updated_at: 2026-10-31T17:34:32Z
---

Optional support for a second Claude Max account, for months when one account's weekly allowance isn't enough. Only the weekly cap matters at the $200 tier; the five-hour window is a non-issue.

## Intended design

- Each account gets its own `CLAUDE_CONFIG_DIR`, logged in once. BobLM already passes `CLAUDE_CONFIG_DIR` through to Claude children (`crates/server/src/persistent.rs`).
- First version: a single manual toggle (Claude 1 / Claude 2) that picks the account for **new** chats. No per-account usage tracking needed; the owner watches the weekly meter.
- A chat stays pinned to the account it started on. Session transcripts live under each config dir and the prompt cache is per account, so switching mid-chat breaks resume or re-sends the whole context at full price.
- Drain sequentially, never alternate: use the account whose regular weekly reset comes first until it is nearly drained, then switch to the other. If both are drained, spend a banked reset on the account that resets later. Sequential use also looks less like account pooling than alternating does.

## Open questions

- [ ] ADR 0018 forbids settings screens. Record the toggle as a deliberate exception in an ADR and choose where it lives (composer or sidebar).
- [ ] Chats pinned to a drained account stall until its reset. Decide whether to wait or move them once. Moving depends on Claude Code resuming from a copied transcript, which is unverified.
- [ ] Optional later: automatic switching. Needs a weekly usage record per account (ADR 0037 keeps samples per chat) that stays fresh while the account is idle. Switch at about 5 to 10% remaining, not 1%, since one long Opus turn can burn more than 1%.

## Risks

- Reported lockouts of multi-account users seem tied to several accounts used from one IP or device. A ban could take out both accounts at once. The owner accepts this risk; keep the Pi driver (ADR 0026) viable as an exit path.
