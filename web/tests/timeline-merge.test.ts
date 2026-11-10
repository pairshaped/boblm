import { test } from "node:test";
import assert from "node:assert/strict";
import { emptyTimeline, projectEvents } from "../src/timeline.ts";
import type { JsonValue, StoredEvent } from "../src/generated/contract.ts";

// Claude Code sends one assistant event per content block, all under one message id.
const block = (seq: number, id: string, b: JsonValue): StoredEvent => ({
  sessionId: "s",
  seq,
  createdAt: 1000 + seq,
  kind: "agent_event",
  payload: { type: "assistant", message: { id, content: [b] } },
});
const text = (seq: number, id: string, t: string) =>
  block(seq, id, { type: "text", text: t });
const thinking = (seq: number, id: string) =>
  block(seq, id, { type: "thinking", thinking: "hm", signature: "sig" });
const tool = (seq: number, id: string, callId: string, name = "Read") =>
  block(seq, id, { type: "tool_use", id: callId, name, input: { path: "x" } });
const delta = (seq: number, t: string): StoredEvent => ({
  sessionId: "s",
  seq,
  createdAt: 1000 + seq,
  kind: "agent_event",
  payload: {
    type: "stream_event",
    event: {
      type: "content_block_delta",
      delta: { type: "text_delta", text: t },
    },
  },
});
const start = (seq: number, id: string): StoredEvent => ({
  sessionId: "s",
  seq,
  createdAt: 1000 + seq,
  kind: "agent_event",
  payload: {
    type: "stream_event",
    event: { type: "message_start", message: { id } },
  },
});

const rowsOf = (events: StoredEvent[]) =>
  projectEvents(emptyTimeline(), events).rows;

test("text followed by a tool call under one message id keeps its text", () => {
  const rows = rowsOf([
    thinking(1, "m"),
    text(2, "m", "I will check."),
    tool(3, "m", "c1"),
  ]);
  assert.equal(rows.length, 1);
  assert.equal(rows[0]!.text, "I will check.");
  assert.equal(rows[0]!.tools?.length, 1);
});

test("several tool calls under one message id are all kept", () => {
  const rows = rowsOf([
    text(1, "m", "Reading both."),
    tool(2, "m", "c1"),
    tool(3, "m", "c2", "Grep"),
    tool(4, "m", "c3", "Bash"),
  ]);
  assert.equal(rows[0]!.text, "Reading both.");
  assert.deepEqual(
    rows[0]!.tools?.map((t) => t.callId),
    ["c1", "c2", "c3"],
  );
});

test("streamed text that the final text block then repeats is shown once", () => {
  const rows = rowsOf([
    start(1, "m"),
    delta(2, "Hel"),
    delta(3, "lo there"),
    thinking(4, "m"),
    text(5, "m", "Hello there"),
    tool(6, "m", "c1"),
  ]);
  assert.equal(rows[0]!.text, "Hello there");
});

test("a text block that arrives after streaming only a part of it completes it", () => {
  const rows = rowsOf([
    start(1, "m"),
    delta(2, "Hel"),
    text(3, "m", "Hello there"),
  ]);
  assert.equal(rows[0]!.text, "Hello there");
});

test("two separate text blocks in one message are both shown", () => {
  const rows = rowsOf([
    text(1, "m", "First part."),
    text(2, "m", "Second part."),
  ]);
  assert.equal(rows[0]!.text, "First part.\nSecond part.");
});

test("projecting the same events twice changes nothing", () => {
  const events = [
    thinking(1, "m"),
    text(2, "m", "Hi."),
    tool(3, "m", "c1"),
    tool(4, "m", "c2"),
  ];
  const once = projectEvents(emptyTimeline(), events);
  const twice = projectEvents(once, events);
  assert.equal(twice.rows[0]!.text, "Hi.");
  assert.equal(twice.rows[0]!.tools?.length, 2);
});

test("one event holding text and a tool call still works", () => {
  const rows = rowsOf([
    block(1, "m", { type: "text", text: "Both." }),
    // the same shape older versions sent: several blocks in one event
    {
      sessionId: "s",
      seq: 2,
      createdAt: 1002,
      kind: "agent_event",
      payload: {
        type: "assistant",
        message: {
          id: "n",
          content: [
            { type: "text", text: "Checking." },
            { type: "tool_use", id: "c9", name: "Read", input: {} },
          ],
        },
      },
    },
  ]);
  assert.equal(rows[1]!.text, "Checking.");
  assert.equal(rows[1]!.tools?.length, 1);
});

test("benign Claude restart is a muted marker and the answer continues", () => {
  const timeline = projectEvents(emptyTimeline(), [
    {
      sessionId: "s",
      seq: 1,
      createdAt: 1,
      kind: "agent_event",
      payload: {
        type: "claude_turn_restart",
        result: {
          type: "result",
          is_error: true,
          errors: ["[ede_diagnostic] result_type=user"],
        },
      },
    },
    text(2, "answer", "Both messages received"),
  ]);
  assert.equal(timeline.rows[0]?.role, "marker");
  assert.equal(
    timeline.rows[0]?.text,
    "Claude restarted its turn to include your new message",
  );
  assert.equal(timeline.rows[1]?.text, "Both messages received");
  assert.ok(
    !timeline.rows.some((row) => row.text.includes("Claude reported an error")),
  );
});
