---
# boblm-s5q9
title: Show API spend this billing cycle
status: todo
type: feature
priority: normal
created_at: 2026-11-22T01:02:34Z
updated_at: 2026-11-22T01:07:20Z
---

Show per-account spend on the Account usage screen for the monthly cycle anchored to each subscription renewal.


Use actual provider-reported API costs for OpenAI and Claude API A. Exclude Claude B. The owner reports that the last Claude A subscription payment was October 26, 2026. Use the 26th as its monthly subscription-cycle anchor. The separate Claude API credit grant on the 27th still governs credit exhaustion. Use the 1st for OpenAI.


Deferred at the owner’s request. The existing OpenAI key returned HTTP 403 for organization costs (missing api.usage.read). The Claude A key returned HTTP 401 for cost analytics (organization admin credential required). Resume when the web server has billing access. Show provider-reported costs for the renewal periods above.
