---
# boblm-724p
title: 'Deferred: runner safety net for detached work'
status: draft
type: feature
priority: normal
created_at: 2026-11-04T07:14:22Z
updated_at: 2026-11-06T18:37:05Z
---

Deferred options from owner decision 73 (2026-11-04); build only if the prompt rules and the Claude background-command hook prove insufficient.

B. Runner safety net for detached work. Every process an agent starts stays inside its chat runner's cgroup, even when detached with &, nohup or setsid. The runner could notice when a process outside any tracked background job exits and wake the chat the way it does for tracked jobs (ADR 0038). Seen live 2026-11-04: A chat ran bin/land as `( ... ) &` inside a Claude background job; the job ended in 4 seconds, bin/land failed later, and nothing woke the chat.
