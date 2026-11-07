import { test } from "node:test";
import assert from "node:assert/strict";
import { emptyTimeline, projectEvents } from "../src/timeline.ts";
import { groupToolRows } from "../src/tool-groups.ts";
import type { JsonValue, StoredEvent } from "../src/generated/contract.ts";

const event = (
  seq: number,
  type: string,
  content: JsonValue[],
): StoredEvent => ({
  sessionId: "s",
  seq,
  createdAt: 1,
  kind: "agent_event",
  payload: { type, message: { id: `m${seq}`, role: type, content } },
});
const body =
  "Base directory for this skill: /home/user/.claude/skills/domain-modeling\n\n# Domain Modeling\n\nActively build and sharpen the model.";
// The order Claude Code produces: the Skill call, its short result, then the skill document
// injected as a separate user message.
const events = [
  event(1, "assistant", [
    {
      type: "tool_use",
      id: "t1",
      name: "Skill",
      input: { skill: "domain-modeling" },
    },
  ]),
  event(2, "user", [
    {
      type: "tool_result",
      tool_use_id: "t1",
      content: "Launching skill: domain-modeling",
    },
  ]),
  event(3, "user", [{ type: "text", text: body }]),
  event(4, "assistant", [{ type: "text", text: "Done." }]),
];

test("the injected skill document never becomes chat text", () => {
  const rows = groupToolRows(projectEvents(emptyTimeline(), events).rows);
  for (const row of rows)
    assert.doesNotMatch(
      row.text,
      /Base directory for this skill/,
      "not shown as a message",
    );
});

test("it is kept in a collapsed detail named for the skill, with the full text inside", () => {
  const rows = groupToolRows(projectEvents(emptyTimeline(), events).rows);
  const tools = rows.flatMap((r) => r.tools ?? []);
  const detail = tools.find((t) => t.text === body);
  assert.ok(detail, "the full document is still available");
  assert.match(detail.label, /domain-modeling/);
  assert.equal(detail.name, "Skill");
});

test("ordinary user text that merely mentions a skill is untouched", () => {
  const rows = projectEvents(emptyTimeline(), [
    event(1, "user", [
      { type: "text", text: "Please base directory for this skill is x" },
    ]),
  ]).rows;
  assert.equal(rows[0]?.text, "Please base directory for this skill is x");
  assert.equal(rows[0]?.tools, undefined);
});

test("the agent's reply after the skill still shows", () => {
  const rows = groupToolRows(projectEvents(emptyTimeline(), events).rows);
  assert.ok(rows.some((r) => r.role === "assistant" && r.text === "Done."));
});
