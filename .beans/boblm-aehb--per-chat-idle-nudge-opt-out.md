---
# boblm-aehb
title: Per-chat idle nudge opt-out
status: completed
type: feature
priority: normal
created_at: 2026-11-19T04:54:33Z
updated_at: 2026-11-19T05:17:08Z
---

Idle nudges are enabled by default. Let the owner turn them off for individual root chats and back on without replaying old opportunities.

- [x] Persist the preference and suppress old opportunities.
- [x] Add the chat header toggle and update ADR 0046.
- [x] Verify with fake drivers and a dev instance.

- [x] Run workspace checks and verify the deployed release.

## Summary of Changes

Root chats now have a durable, default-on idle nudge preference in the chat header. Opting out drops pending opportunities and cancels unclaimed queued nudges. Re-enabling applies to future turn results.

Verified with the focused delivery tests, the full workspace checks with four test threads, a two-tab browser smoke test and an upgrade with an existing runner. Backed up the database before migration 057 and verified the running web release.
