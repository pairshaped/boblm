import { test } from "node:test";
import assert from "node:assert/strict";
import { initial, update, type Model } from "../src/app.ts";
import type { SessionInfo } from "../src/generated/contract.ts";

const chat = (id: string, archived = false): SessionInfo => ({
  name: id,
  sessionId: id,
  workingDirectory: "/w",
  agentSessionId: null,
  agent: "claude" as const,
  projectId: "p",
  createdAt: 1,
  status: {
    selectionPending: false,
    queueHeld: false,
    running: false,
    turnActive: false,
    stopping: false,
    queued: 0,
    dispatching: 0,
    uncertain: 0,
    rejected: 0,
    archived,
    model: "sonnet" as const,
    effort: null,
  },
});
const project = { projectId: "p", name: "p", path: "/p", createdAt: 1 };
const models = [
  {
    model: "sonnet",
    label: "sonnet",
    agent: "claude" as const,
    efforts: ["low"],
  },
];
const base = (active: string): Model => ({
  ...initial("t"),
  projects: [project],
  sessions: [chat("open"), chat(active, true)],
  models,
  active,
});
const deleteReply = (sessionId: string) => ({
  type: "reply" as const,
  name: "deleteSession",
  sessionId,
  outcome: { kind: "replied", reply: {} } as never,
});
const lateFailure = (sessionId: string) => ({
  type: "reply" as const,
  name: "archiveSession",
  sessionId,
  outcome: {
    kind: "rejected",
    error: {
      code: "not_found",
      message: `There is no chat with ID "${sessionId}" on this server.`,
    },
  } as never,
});

test("deleting the active archived chat forgets it and ignores its late replies", () => {
  const [deleted, effects] = update(base("gone"), deleteReply("gone"));
  assert.equal(deleted.active, null);
  assert.ok(
    !effects.some(
      (effect) => "sessionId" in effect && effect.sessionId === "gone",
    ),
    "the delete reply starts nothing for the deleted chat",
  );
  const [late] = update(deleted, lateFailure("gone"));
  assert.equal(
    late.error,
    "",
    "a late reply for a forgotten chat must not show the server's not-found error",
  );
});

test("deleting another archived chat keeps the open chat and ignores late replies", () => {
  const [deleted] = update(base("open"), deleteReply("gone"));
  assert.equal(deleted.active, "open");
  const [late] = update(deleted, lateFailure("gone"));
  assert.equal(late.error, "");
  assert.equal(late.active, "open");
});
