import { test } from "node:test";
import assert from "node:assert/strict";
import { subagentsFrom } from "../src/subagents.ts";
import type { ChatRow } from "../src/timeline.ts";

let seq = 0;
const row = (
  at: number,
  tool: {
    name: string;
    kind: "call" | "result";
    callId?: string;
    error?: boolean;
    block: object;
  },
): ChatRow => ({
  key: `r${++seq}`,
  role: tool.kind === "call" ? "assistant" : "tool",
  text: "",
  seq,
  revision: seq,
  at,
  tools: [
    {
      key: `${tool.kind}:0`,
      name: tool.name,
      ...(tool.callId ? { callId: tool.callId } : {}),
      kind: tool.kind,
      error: tool.error ?? false,
      label: tool.name,
      text: JSON.stringify(tool.block),
    },
  ],
});
const claudeCall = (
  at: number,
  id: string,
  description: string,
  name = "Agent",
) =>
  row(at, {
    name,
    kind: "call",
    callId: id,
    block: {
      type: "tool_use",
      id,
      name,
      input: { description, subagent_type: "general-purpose", prompt: "x" },
    },
  });
const claudeResult = (at: number, id: string, error = false) =>
  row(at, {
    name: "Agent",
    kind: "result",
    callId: id,
    error,
    block: {
      type: "tool_result",
      tool_use_id: id,
      is_error: error,
      content: "done",
    },
  });
const codex = (at: number, kind: string, path: string, thread: string) =>
  row(at, {
    name: "subAgentActivity",
    kind: "call",
    callId: `c${at}`,
    block: {
      type: "tool_use",
      name: "subAgentActivity",
      input: {
        agentPath: path,
        agentThreadId: thread,
        kind,
        type: "subAgentActivity",
      },
    },
  });

test("a Claude Agent call is active until its result arrives, then done", () => {
  const running = subagentsFrom([
    claudeCall(100, "t1", "Review cutover adoption"),
  ]);
  assert.deepEqual(
    running.map((s) => [s.title, s.state, s.startedAt]),
    [["Review cutover adoption", "active", 100]],
  );
  const finished = subagentsFrom([
    claudeCall(100, "t1", "Review cutover adoption"),
    claudeResult(250, "t1"),
  ]);
  assert.deepEqual(
    finished.map((s) => [s.state, s.endedAt]),
    [["done", 250]],
  );
});

test("the older Task tool name counts, and an errored result is failed", () => {
  const list = subagentsFrom([
    claudeCall(10, "t1", "Old name", "Task"),
    claudeResult(20, "t1", true),
  ]);
  assert.equal(list[0]?.title, "Old name");
  assert.equal(list[0]?.state, "failed");
});

test("a Codex subagent starts and completes by its thread, named from its path", () => {
  const list = subagentsFrom([
    codex(10, "started", "/root/scoring_evaluation", "th1"),
    codex(20, "interacted", "/root", "main"),
    codex(30, "started", "/root/scoring_plan_review", "th2"),
    codex(40, "completed", "/root/scoring_evaluation", "th1"),
  ]);
  assert.deepEqual(
    list.map((s) => [s.title, s.state]),
    [
      ["Scoring plan review", "active"],
      ["Scoring evaluation", "done"],
    ],
    "active first; the parent agent at /root is not a subagent",
  );
});

test("a completion whose start was not loaded still appears, as done", () => {
  const list = subagentsFrom([codex(40, "completed", "/root/late_one", "th9")]);
  assert.deepEqual(
    list.map((s) => [s.title, s.state, s.endedAt]),
    [["Late one", "done", 40]],
  );
});

test("done subagents list newest first, and nothing else in a chat counts", () => {
  const list = subagentsFrom([
    claudeCall(10, "a", "First"),
    claudeResult(11, "a"),
    claudeCall(20, "b", "Second"),
    claudeResult(21, "b"),
    row(30, {
      name: "Bash",
      kind: "call",
      callId: "x",
      block: { type: "tool_use", name: "Bash", input: {} },
    }),
    {
      key: "plain",
      role: "assistant",
      text: "hello",
      seq: 99,
      revision: 99,
      at: 40,
    },
  ]);
  assert.deepEqual(
    list.map((s) => s.title),
    ["Second", "First"],
  );
});

test("damaged tool text is skipped, not thrown on", () => {
  const broken: ChatRow = {
    key: "bad",
    role: "assistant",
    text: "",
    seq: 1,
    revision: 1,
    at: 1,
    tools: [
      {
        key: "k",
        name: "Agent",
        callId: "z",
        kind: "call",
        error: false,
        label: "Agent",
        text: "{not json",
      },
    ],
  };
  assert.deepEqual(subagentsFrom([broken]), []);
});
