import { test } from "node:test";
import assert from "node:assert/strict";
import { initial, update, type Model } from "../src/app.ts";
import type { SessionInfo } from "../src/generated/contract.ts";

const status = {
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
};
const chat = (id: string, extra: Partial<SessionInfo> = {}): SessionInfo => ({
  name: id,
  sessionId: id,
  workingDirectory: "/w",
  agentSessionId: null,
  agent: "claude" as const,
  projectId: "p",
  createdAt: 1,
  status,
  ...extra,
});
const root = (open: number) =>
  chat("root", {
    subagents: { open, questions: 0, failures: 0, reportFailures: 0 },
  });
const child = chat("kid", { parentSessionId: "root", rootSessionId: "root" });
const projects = [{ projectId: "p", name: "W", path: "/p", createdAt: 1 }];
const models = [
  {
    model: "sonnet",
    label: "sonnet",
    agent: "claude" as const,
    efforts: ["low"],
  },
];
const start = (): Model =>
  initial("t", {}, {}, {}, null, {}, {}, false, null, {
    sessions: [root(1), child],
    projects,
    models,
  });
const asksForList = (effects: ReadonlyArray<{ type: string }>) =>
  effects.some(
    (e) =>
      e.type === "request" && (e as { name?: string }).name === "listSessions",
  );
const listEvent = (
  seq: number,
  kind: string,
  sessionId: string,
  payload: unknown,
) =>
  ({
    type: "listEvent",
    event: { seq, sessionId, kind, createdAt: 1, payload },
  }) as never;
// The server's snapshot is the owner of the counts; the reply carries them.
const reply = (model: Model, sessions: SessionInfo[]) =>
  update(model, {
    type: "reply",
    name: "listSessions",
    sessionId: "",
    orderGeneration: model.orderGeneration,
    outcome: {
      kind: "replied",
      reply: { sessions, projects, models, cursor: model.listCursor },
    },
  } as never)[0];

test("archiving a child refreshes the root's open count to 0", () => {
  const [next, effects] = update(
    start(),
    listEvent(1, "archived", "kid", { archived: true }),
  );
  assert.ok(asksForList(effects));
  const after = reply(next, [
    root(0),
    { ...child, status: { ...status, archived: true } },
  ]);
  assert.equal(
    after.sessions.find((s) => s.sessionId === "root")!.subagents!.open,
    0,
  );
});

test("reopening a child refreshes the root's open count", () => {
  const archived = { ...child, status: { ...status, archived: true } };
  const model = { ...start(), sessions: [root(0), archived] };
  const [next, effects] = update(
    model,
    listEvent(1, "archived", "kid", { archived: false }),
  );
  assert.ok(asksForList(effects));
  const after = reply(next, [root(1), child]);
  assert.equal(
    after.sessions.find((s) => s.sessionId === "root")!.subagents!.open,
    1,
  );
});

test("deleting a child refreshes the root's open count", () => {
  const [next, effects] = update(start(), listEvent(1, "deleted", "kid", {}));
  assert.ok(asksForList(effects));
  const after = reply(next, [root(0)]);
  assert.equal(
    after.sessions.find((s) => s.sessionId === "root")!.subagents!.open,
    0,
  );
});

test("a child's turn ending refreshes the root's failure counts", () => {
  const model = {
    ...start(),
    sessions: [root(1), { ...child, status: { ...status, turnActive: true } }],
  };
  const [, effects] = update(
    model,
    listEvent(1, "state_summary", "kid", status),
  );
  assert.ok(asksForList(effects));
});

test("a child's unchanged turn state does not refetch the list", () => {
  const [, effects] = update(
    start(),
    listEvent(1, "state_summary", "kid", { ...status, queued: 1 }),
  );
  assert.ok(!asksForList(effects));
});

test("spawning a child refreshes the list", () => {
  const [, effects] = update(
    start(),
    listEvent(1, "created", "new", { name: "n" }),
  );
  assert.ok(asksForList(effects));
});

const requestIds = (effects: ReadonlyArray<unknown>) =>
  effects.filter(
    (e) =>
      (e as { type: string; name?: string }).type === "request" &&
      (e as { name?: string }).name === "listSessions",
  ) as ReadonlyArray<{ listRequest?: number }>;
const listReply = (
  model: Model,
  listRequest: number | undefined,
  sessions: SessionInfo[],
  cursor: number,
) =>
  update(model, {
    type: "reply",
    name: "listSessions",
    sessionId: "",
    orderGeneration: model.orderGeneration,
    ...(listRequest === undefined ? {} : { listRequest }),
    outcome: { kind: "replied", reply: { sessions, projects, models, cursor } },
  } as never);

test("a question change under a root refreshes the root's summary", () => {
  const [, effects] = update(
    start(),
    listEvent(1, "subagents_changed", "root", {}),
  );
  assert.ok(asksForList(effects));
});

test("refetches coalesce: one in flight, one queued follow-up, however many events arrive", () => {
  let model = start();
  let effects: ReadonlyArray<unknown>;
  [model, effects] = update(
    model,
    listEvent(1, "subagents_changed", "root", {}),
  );
  const first = requestIds(effects);
  assert.equal(first.length, 1);
  for (let seq = 2; seq <= 6; seq++) {
    [model, effects] = update(
      model,
      listEvent(seq, "subagents_changed", "root", {}),
    );
    assert.equal(
      requestIds(effects).length,
      0,
      `event ${seq} must not start a request`,
    );
  }
  // The reply to the first request releases exactly one follow-up.
  [model, effects] = listReply(
    model,
    first[0]!.listRequest,
    [root(1), child],
    6,
  );
  const follow = requestIds(effects);
  assert.equal(follow.length, 1);
  assert.notEqual(follow[0]!.listRequest, first[0]!.listRequest);
  // With nothing queued behind it, the follow-up's reply ends the chain.
  [model, effects] = listReply(
    model,
    follow[0]!.listRequest,
    [root(1), child],
    6,
  );
  assert.equal(requestIds(effects).length, 0);
});

test("an older list reply cannot reset the cursor or overwrite a newer list", () => {
  let model = start();
  let effects: ReadonlyArray<unknown>;
  [model, effects] = update(
    model,
    listEvent(1, "subagents_changed", "root", {}),
  );
  const old = requestIds(effects)[0]!.listRequest;
  // A reload requests a fresh list, so the earlier request is now superseded.
  [model, effects] = update(model, { type: "refresh" } as never);
  const current = requestIds(effects)[0]!.listRequest;
  assert.notEqual(old, current);
  [model] = listReply(model, current, [root(2), child], 40);
  assert.equal(model.listCursor, 40);
  const [after, none] = listReply(model, old, [root(0)], 3);
  assert.equal(after.listCursor, 40);
  assert.equal(after.sessions.length, 2);
  assert.equal(requestIds(none).length, 0);
});
