import { test } from "node:test";
import assert from "node:assert/strict";
import { emptyTimeline, projectEvents } from "../src/timeline.ts";
import { groupToolRows } from "../src/tool-groups.ts";
import type { JsonValue, StoredEvent } from "../src/generated/contract.ts";

const summary =
  "This session is being continued from a previous conversation that ran out of context. The summary below covers the earlier portion of the conversation.\n\n<tools-used-before-summary>Read, Bash</tools-used-before-summary>\nSummary:\n1. Primary Request and Intent: Keep working.";
const event = (
  seq: number,
  content: JsonValue,
  type = "user",
): StoredEvent => ({
  sessionId: "s",
  seq,
  createdAt: seq,
  kind: "agent_event",
  payload: { type, message: { id: `m${seq}`, content } },
});
const boundary: StoredEvent = {
  sessionId: "s",
  seq: 1,
  createdAt: 1,
  kind: "agent_event",
  payload: { type: "system", subtype: "compact_boundary" },
};

for (const content of [summary, [{ type: "text", text: summary }]]) {
  test(`Claude compaction summary stays in its own disclosure (${typeof content})`, () => {
    const events = [
      boundary,
      event(2, content),
      event(
        3,
        [
          {
            type: "tool_use",
            id: "read",
            name: "Read",
            input: { path: "file.rs" },
          },
        ],
        "assistant",
      ),
    ];
    const history = groupToolRows(projectEvents(emptyTimeline(), events).rows);
    let live = emptyTimeline();
    for (const e of events) live = projectEvents(live, [e]);
    assert.deepEqual(groupToolRows(live.rows), history);
    assert.match(history[0]!.text, /Conversation compacted/);
    assert.equal(history[1]!.text, "");
    assert.equal(history[1]!.toolGroupLabel, undefined);
    assert.equal(history[1]!.tools?.[0]?.label, "Compaction summary");
    assert.equal(history[1]!.tools?.[0]?.text, summary);
    assert.equal(history[2]!.tools?.[0]?.name, "Read");
  });
}

test("owner messages and assistant replies quoting the summary remain chat text", () => {
  const owner: StoredEvent = {
    sessionId: "s",
    seq: 1,
    createdAt: 1,
    kind: "message_appended",
    payload: {
      uuid: "owner",
      role: "user",
      content: summary,
      delivery_state: "started",
    },
  };
  const rows = projectEvents(emptyTimeline(), [
    owner,
    event(2, [{ type: "text", text: summary }], "assistant"),
  ]).rows;
  assert.deepEqual(
    rows.map((r) => r.text),
    [summary, summary],
  );
  assert.ok(rows.every((r) => !r.tools?.length));
});

test("ordinary provider user text remains visible", () => {
  const rows = projectEvents(emptyTimeline(), [
    event(1, [{ type: "text", text: "Let's discuss conversation summaries." }]),
  ]).rows;
  assert.equal(rows[0]!.text, "Let's discuss conversation summaries.");
  assert.equal(rows[0]!.tools, undefined);
});
