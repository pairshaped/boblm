---
# boblm-rdaj
title: Antigravity accounts and quota meters
status: todo
type: feature
created_at: 2026-11-20T13:56:48Z
updated_at: 2026-11-20T13:56:48Z
parent: boblm-dv29
---

Pin Antigravity chats to an account key and show its five-hour and weekly allowances (ADR 0037). Quotas are per model group (Gemini vs Claude/GPT), not per account. Depends on how agy stores credentials and whether its config home can be isolated per account (see the live capture). Role routing and replacing Claude plan B are out of scope until the owner decides.

- [ ] Account identity source
- [ ] Quota reader (ACP data, or a tolerant parser for agy -p /usage)
- [ ] Meters in the composer
