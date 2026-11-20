---
# boblm-pe01
title: Install agy, sign in, and capture the live ACP protocol
status: todo
type: task
priority: normal
created_at: 2026-11-20T13:56:48Z
updated_at: 2026-11-20T13:56:48Z
parent: boblm-dv29
blocking:
    - boblm-rdaj
---

Blocked on the owner: install agy (curl -fsSL https://antigravity.google/cli/install.sh | bash -s -- --skip-path --skip-aliases) and sign in once (agy acp --login works over SSH).

Then work through the 13 'Verify live' items in docs/research/antigravity-cli.md and record the observations in docs/live-antigravity-protocol.md, in the style of docs/live-pi-protocol.md.

- [ ] Owner subscribes to Google AI Ultra, installs agy and signs in
- [ ] Credential storage: where the token lands without a Secret Service, and whether the boblm systemd user service can use it
- [ ] Config-home override for account isolation
- [ ] initialize / session/new replies, config option IDs, available commands
- [ ] Prompt, steer, cancel, load/resume transcripts
- [ ] HTTP MCP with auth header (--process-per-session?)
- [ ] Unattended settings: permissions and artifactReviewPolicy
- [ ] Compaction behavior
- [ ] Quota source: agy -p /usage output or ACP data
- [ ] agy models on the owner's plan
