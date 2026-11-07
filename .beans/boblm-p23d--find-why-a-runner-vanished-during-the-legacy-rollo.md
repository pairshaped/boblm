---
# boblm-p23d
title: Find why a runner vanished during the legacy rollout
status: todo
type: bug
created_at: 2026-10-30T17:54:30Z
updated_at: 2026-10-30T17:54:30Z
---

A coordinating chat runner unit and socket were absent after persistent rollout. Journal retained only unit start, no useful exit cause. Manually starting only missing unit restored history API and stayed active with zero restarts. Root exit cause remains unproven. History fix and explicit journal output passed bin/check, dev smoke. Investigate startup lock contention, clean exit and supervisor recovery with a reproducible dev harness; do not infer cause from database-locked errors alone.
