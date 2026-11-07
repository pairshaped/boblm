import { test } from "node:test";
import assert from "node:assert/strict";
import { emptyTimeline, projectEvents } from "../src/timeline.ts";
import { groupToolRows } from "../src/tool-groups.ts";
import type { StoredEvent, JsonValue } from "../src/generated/contract.ts";

const event = (
  seq: number,
  content: JsonValue[],
  role = "assistant",
): StoredEvent => ({
  sessionId: "s",
  seq,
  createdAt: 1,
  kind: "agent_event",
  payload: { type: role, message: { id: `m${seq}`, content } },
});
const call = (id: string, name = "Read") => ({
  type: "tool_use",
  id,
  name,
  input: { path: "file.txt" },
});
const result = (id: string, error = false) => ({
  type: "tool_result",
  tool_use_id: id,
  content: "output",
  is_error: error,
});

test("routine calls and results form one summary without discarding detail", () => {
  const events = [
    event(1, [call("a")]),
    event(2, [result("a")], "user"),
    event(3, [call("b", "Bash")]),
    event(4, [result("b")], "user"),
  ];
  const rows = groupToolRows(projectEvents(emptyTimeline(), events).rows);
  assert.equal(rows.length, 1);
  assert.match(rows[0]!.toolGroupLabel!, /2 tool calls/);
  assert.match(rows[0]!.toolGroupLabel!, /Read/);
  assert.equal(rows[0]!.tools!.length, 4);
  assert.deepEqual(rows[0]!.sourceKeys, ["m1", "m2", "m3", "m4"]);
});

test("text, errors and interactive tools break groups", () => {
  const events = [
    event(1, [call("a")]),
    event(2, [result("a", true)], "user"),
    event(3, [call("b", "request_user_input")]),
    event(4, [{ type: "text", text: "Progress" }]),
    event(5, [call("c", "spawn_agent")]),
  ];
  const rows = groupToolRows(projectEvents(emptyTimeline(), events).rows);
  assert.equal(rows.length, 5);
  assert.ok(rows.slice(1).every((r) => !r.toolGroupLabel));
  assert.equal(rows[1]!.tools![0]!.error, true);
});

test("live updates and history snapshots produce the same groups for each agent", () => {
  const events = [
    event(1, [call("a", "shell")]),
    event(2, [result("a")], "user"),
    event(3, [call("b", "read")]),
  ];
  for (const agent of ["Claude", "Codex", "Pi"]) {
    let live = emptyTimeline();
    for (const e of events) live = projectEvents(live, [e], agent);
    assert.deepEqual(
      groupToolRows(live.rows),
      groupToolRows(projectEvents(emptyTimeline(), events, agent).rows),
    );
    const before = groupToolRows(live.rows)[0]!;
    live = projectEvents(live, [event(4, [result("b")], "user")], agent);
    assert.equal(groupToolRows(live.rows)[0]!.key, before.key);
  }
});

test("results without their calls remain available and are not counted as new calls", () => {
  const rows = groupToolRows(
    projectEvents(emptyTimeline(), [
      event(1, [result("a"), result("b")], "user"),
    ]).rows,
  );
  assert.match(rows[0]!.toolGroupLabel!, /2 tool results/);
  assert.equal(rows[0]!.tools!.length, 2);
});

test("namespaced credential and peer actions remain standalone", () => {
  const events = [
    "mcp__boblm__request_sops_secret",
    "boblm.spawn_peer",
    "boblm.ping_peer",
  ].map((name, i) => event(i + 1, [call(String(i), name)]));
  const rows = groupToolRows(projectEvents(emptyTimeline(), events).rows);
  assert.equal(rows.length, 3);
  assert.ok(rows.every((row) => !row.toolGroupLabel));
});

test("thinking blocks never appear as JSON chat text", () => {
  const thinking = { type: "thinking", thinking: "hmm", signature: "EqQBCkYI" };
  const redacted = { type: "redacted_thinking", data: "EncryptedBlob" };
  const rows = projectEvents(emptyTimeline(), [
    event(1, [thinking, redacted]),
    event(2, [{ type: "text", text: "Hello" }]),
  ]).rows;
  assert.equal(rows[0]!.text, "");
  assert.equal(rows[1]!.text, "Hello");
});
