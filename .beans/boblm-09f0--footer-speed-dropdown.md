---
# boblm-09f0
title: Footer speed dropdown
status: completed
type: feature
priority: normal
created_at: 2026-11-21T17:20:24Z
updated_at: 2026-11-21T17:49:31Z
---

Add a speed selector immediately after reasoning effort in the chat footer.


## Summary of Changes

The footer saves speed per Codex chat and applies it to the next turn. Normal (1x) explicitly requests the standard service tier. Fast and Ultrafast appear according to the selected model. Verified the options and persistence in the dev browser and the tier sent to the fake Codex provider.

Built the release and verified the pre-058 backup. Production runner replacement must wait for active chats to become idle.


Full workspace verification passed with bin/check. The production rollout is pending.


The follow-up renames the default option to Normal and adds optional speed to spawn_peer and spawn_subagent. The spawn transaction stores the requested tier before queuing the first message. The server checks runner support before sending a speed change. Reproduced the EOF with a pre-058 runner, then verified disabled Normal on that runner and a saved Fast selection after upgrading it.


Deployed the follow-up after bin/check passed. Verified that the live peer and subagent tools advertise optional speed and this chat reports support for speed changes.
