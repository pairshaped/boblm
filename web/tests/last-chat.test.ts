import { test } from "node:test";
import assert from "node:assert/strict";
import { initial, update, type Model } from "../src/app.ts";
import type { SessionInfo } from "../src/generated/contract.ts";

const chat = (id: string): SessionInfo => ({
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
    archived: false,
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
const listReply = (...ids: string[]) => ({
  type: "reply" as const,
  name: "listSessions",
  sessionId: "",
  outcome: {
    kind: "replied",
    reply: { sessions: ids.map(chat), models, projects: [project] },
  } as never,
});
// The page was just loaded: nothing is known yet, and the saved chat is waiting for the list.
const loaded = (saved: string | null): Model =>
  initial("t", {}, {}, {}, null, {}, {}, false, saved);

test("with nothing saved, a reload starts empty as before", () => {
  assert.equal(initial("t").restoreChat, null);
  const [next] = update(loaded(null), listReply("s1", "s2"));
  assert.equal(next.active, null);
});

test("the saved chat reopens once the list arrives", () => {
  const [next, effects] = update(loaded("s2"), listReply("s1", "s2"));
  assert.equal(next.active, "s2");
  assert.equal(next.restoreChat, null, "it is restored only once");
  assert.ok(
    effects.some((e) => e.type === "composer" && e.sessionId === "s2"),
    "its draft is put back like any other selection",
  );
});

test("a saved chat that no longer exists is dropped quietly", () => {
  const [next, effects] = update(loaded("gone"), listReply("s1", "s2"));
  assert.equal(next.active, null);
  assert.equal(next.restoreChat, null);
  assert.equal(next.error, "");
  assert.deepEqual(
    effects.filter((e) => e.type === "persistLastChat"),
    [{ type: "persistLastChat", id: null }],
  );
});

test("a chat the owner already picked is not replaced by the saved one", () => {
  const picked = { ...loaded("s2"), active: "s1" };
  const [next] = update(picked, listReply("s1", "s2"));
  assert.equal(next.active, "s1");
  assert.equal(next.restoreChat, null);
});

test("choosing, creating and deleting a chat keep the saved one current", () => {
  const base: Model = {
    ...initial("t"),
    projects: [project],
    sessions: [chat("s1"), chat("s2")],
    models,
  };
  const [, selecting] = update(base, { type: "select", id: "s2" });
  assert.deepEqual(
    selecting.filter((e) => e.type === "persistLastChat"),
    [{ type: "persistLastChat", id: "s2" }],
  );
  const [, created] = update(base, {
    type: "reply",
    name: "createSession",
    sessionId: "n1",
    outcome: { kind: "replied", reply: chat("n1") } as never,
  });
  assert.deepEqual(
    created.filter((e) => e.type === "persistLastChat"),
    [{ type: "persistLastChat", id: "n1" }],
  );
  const open: Model = { ...base, active: "s1" };
  const [gone, deleted] = update(open, {
    type: "reply",
    name: "deleteSession",
    sessionId: "s1",
    outcome: { kind: "replied", reply: {} } as never,
  });
  assert.equal(gone.active, null);
  assert.deepEqual(
    deleted.filter((e) => e.type === "persistLastChat"),
    [{ type: "persistLastChat", id: null }],
  );
});

test("locking remembers the open chat so unlocking returns to it", () => {
  const open: Model = {
    ...initial("t"),
    sessions: [chat("s1")],
    projects: [project],
    models,
    active: "s1",
  };
  const [locked] = update(open, { type: "logout" });
  assert.equal(locked.active, null);
  assert.equal(locked.restoreChat, "s1");
});
