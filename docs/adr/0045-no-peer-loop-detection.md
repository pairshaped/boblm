# 0045: No peer loop detection

BobLM does not detect or interrupt back-and-forth messaging between chats. Runaway
work is bounded by mechanisms that measure work, not conversation shape: one level
of subagents with at most three open per root (ADR 0043), the per-round runaway
limit on subagents, which pauses one and asks its parent to decide (ADR 0043), and
per-minute token headroom for API reviewers.

## Why not a loop detector

The obvious rule counts messages between two chats with no owner message and no
real work (tool calls other than messaging) in between, then warns or pauses the
pair. Measured against the full message history on 2026-11-06, that rule finds
many long runs and no actual loops:

- 163 runs of three or more inbound messages from one chat with no work by the
  receiver in between; 12 reached six or more.
- The longest was a delivery bug that sent ten identical subagent reports, which
  is fixed in the report outbox, not something a detector should catch.
- Almost every other long run was work chats sending a coordinating chat a series
  of status updates and landing requests. The coordinator reads and relays them
  without other tool work. That is a hub doing its job.

A detector on this rule would routinely warn or pause the hub during normal
operation. A version good enough to trust would need messages flowing both ways
between the same pair, no work on either side, probably short or repetitive
content, and likely an exemption for hub chats. With no real loop to catch, that
complexity is not justified.

Revisit if a real loop appears: two chats exchanging messages that do not advance
any work. The first response should then be a warning to both agents that they
may be ping-ponging, with a pause and an owner question only if the warning is
ignored.
