---
# boblm-gswe
title: ADR and antigravity-driver crate over ACP
status: todo
type: feature
priority: normal
created_at: 2026-11-20T13:56:48Z
updated_at: 2026-11-20T13:56:48Z
parent: boblm-dv29
blocked_by:
    - boblm-pe01
---

Write the ADR from the live findings, then add crates/antigravity-driver behind the runner's Driver interface, following the Codex and Pi drivers: JSONL framing, request correlation, consumption only from the echoed user message, steer via session/prompt, interrupt via session/cancel, resume, /clear, compaction if available, Claude-shaped event translation, peer tools as an HTTP MCP server with the capability in a header, unattended permission handling, process-group cleanup, and fake-child tests. Tolerate unknown fields and update kinds.

- [ ] ADR
- [ ] Driver crate and fake-child tests
- [ ] Agent enum, catalog entries (MODELS) for Hydrogen, Neon and Carbon (tiers like Haiku/Sonnet/Opus and Luna/Sol/Astra; labels hydrogen, neon, carbon; efforts low-xhigh for Hydrogen and Neon, low-high for Carbon; Argon and Orchestrator left out, see research note), regenerate contract
- [ ] Server spawn wiring and runner integration
- [ ] Browser label, model grouping and effort checks
