import { test } from "node:test";
import assert from "node:assert/strict";
import { cleanTerminalText } from "../src/terminal-text.ts";
import { emptyTimeline, projectEvents } from "../src/timeline.ts";
import type { JsonValue, StoredEvent } from "../src/generated/contract.ts";

test("colored output keeps its text and loses its color codes", () => {
  assert.equal(
    cleanTerminalText("\u001b[31mred\u001b[0m and \u001b[1;32mgreen\u001b[0m"),
    "red and green",
  );
});

test("an OSC title disappears with either terminator", () => {
  assert.equal(cleanTerminalText("\u001b]0;window title\u0007done"), "done");
  assert.equal(cleanTerminalText("\u001b]0;window title\u001b\\done"), "done");
});

test("a result that is only an OSC title becomes empty", () => {
  assert.equal(cleanTerminalText("\u001b]0;\u0007"), "");
});

test("other two-character escapes and stray controls are removed", () => {
  assert.equal(cleanTerminalText("\u001b7saved\u001b8"), "saved");
  assert.equal(cleanTerminalText("a\u0000b\u0007c"), "abc");
});

test("carriage returns are normalized", () => {
  assert.equal(cleanTerminalText("one\r\ntwo"), "one\ntwo");
  assert.equal(cleanTerminalText("one\rtwo"), "onetwo");
});

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

const toolsOf = (events: StoredEvent[]) =>
  projectEvents(emptyTimeline(), events).rows.flatMap((row) => row.tools ?? []);

test("timeline cleans tool result text but leaves tool calls untouched", () => {
  const call = {
    type: "tool_use",
    id: "t1",
    name: "Bash",
    input: { command: "printf '\u001b]0;\u0007'" },
  };
  const result = {
    type: "tool_result",
    tool_use_id: "t1",
    content: [
      { type: "text", text: "\u001b[32mok\u001b[0m" },
      { type: "text", text: "\u001b]0;title\u0007" },
    ],
  };
  const events = [event(1, "assistant", [call]), event(2, "user", [result])];
  const before = JSON.stringify(events);
  const tools = toolsOf(events);
  const shownCall = tools.find((tool) => tool.kind === "call");
  const shownResult = tools.find((tool) => tool.kind === "result");

  assert.equal(shownCall?.text, JSON.stringify(call, null, 2));
  assert.match(shownCall?.text ?? "", /\\u001b/);
  assert.match(shownResult?.text ?? "", /"text": "ok"/);
  assert.doesNotMatch(shownResult?.text ?? "", /\\u001b/);
  assert.equal(JSON.stringify(events), before);
});

test("timeline renders a result that is only an OSC title as empty content", () => {
  const events = [
    event(1, "user", [
      {
        type: "tool_result",
        tool_use_id: "t1",
        content: "\u001b]0;\u0007",
      },
    ]),
  ];
  const result = toolsOf(events).find((tool) => tool.kind === "result");
  const shown = JSON.parse(result?.text ?? "{}") as { content?: unknown };

  assert.equal(shown.content, "");
});
