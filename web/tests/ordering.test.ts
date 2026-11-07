import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { mountProgram } from "@pairshaped/hypertea/testing";
import {
  initial,
  program,
  update,
  visibleChats,
  type Model,
  type DragItem,
} from "../src/app.ts";
import { parseSequence, type SessionInfo } from "../src/generated/contract.ts";

const project = (id: string) => ({
  projectId: id,
  name: id,
  path: `/work/${id}`,
  createdAt: 1,
});
const chat = (id: string, projectId: string): SessionInfo => ({
  sessionId: id,
  projectId,
  name: id,
  agent: "pi",
  workingDirectory: `/work/${projectId}`,
  agentSessionId: null,
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
    model: "deepseek-ai/DeepSeek-V4.1-Flash-1017",
    effort: null,
  },
});
const base: Model = {
  ...initial("fixture"),
  projects: [project("p1"), project("p2"), project("p3")],
  sessions: [
    chat("a", "p1"),
    chat("x", "p2"),
    chat("b", "p1"),
    chat("y", "p2"),
    chat("c", "p1"),
  ],
};
const p = (id: string): DragItem => ({ kind: "project", id });
const c = (id: string, projectId = "p1"): DragItem => ({
  kind: "chat",
  id,
  projectId,
});
const ids = (m: Model) => m.projects.map((p) => p.projectId);
const chats = (m: Model) => m.sessions.map((s) => s.sessionId);
function move(model: Model, item: DragItem, target: DragItem, after: boolean) {
  const [dragging] = update(model, { type: "dragStart", item });
  return update(dragging, { type: "drop", target, after });
}

test("project drops move before or after a target without swapping or changing chats", () => {
  const [before, effects] = move(base, p("p3"), p("p1"), false);
  assert.deepEqual(ids(before), ["p3", "p1", "p2"]);
  assert.equal(before.sessions, base.sessions);
  assert.equal(before.dragging, null);
  assert.equal(before.dropTarget, null);
  assert.deepEqual(effects, [
    {
      type: "request",
      name: "reorderProjects",
      sessionId: "",
      orderGeneration: 1,
      body: { projectIds: ["p3", "p1", "p2"] },
    },
  ]);
  const [after] = move(base, p("p1"), p("p2"), true);
  assert.deepEqual(ids(after), ["p2", "p1", "p3"]);
});

test("chat drops permute only their project's held slots and send the exact order", () => {
  const [after, effects] = move(base, c("a"), c("c"), true);
  assert.deepEqual(chats(after), ["b", "x", "c", "y", "a"]);
  assert.equal(after.projects, base.projects);
  assert.deepEqual(effects, [
    {
      type: "request",
      name: "reorderChats",
      sessionId: "",
      orderGeneration: 1,
      body: { projectId: "p1", sessionIds: ["b", "c", "a"] },
    },
  ]);
  assert.deepEqual(chats(move(base, c("c"), c("a"), false)[0]), [
    "c",
    "x",
    "a",
    "y",
    "b",
  ]);
});

test("self, unchanged, foreign-project, mismatched and stale drops send nothing", () => {
  for (const [item, target, after] of [
    [p("p1"), p("p1"), true],
    [p("p1"), p("p2"), false],
    [p("p2"), p("p1"), true],
    [c("a"), c("a"), false],
    [c("a"), c("b"), false],
    [c("a"), c("x", "p2"), true],
    [p("p1"), c("a"), true],
    [c("a"), p("p1"), true],
    [p("missing"), p("p1"), true],
    [p("p1"), p("missing"), true],
    [c("x", "p1"), c("b"), true],
  ] as const) {
    const [model, effects] = move(base, item, target, after);
    assert.deepEqual(ids(model), ids(base));
    assert.deepEqual(chats(model), chats(base));
    assert.deepEqual(effects, []);
    assert.equal(model.dragging, null);
    assert.equal(model.dropTarget, null);
  }
});

test("drag hover only highlights a matching target and drag end clears it", () => {
  let [model] = update(base, { type: "dragStart", item: c("a") });
  [model] = update(model, { type: "dragOver", target: c("b"), after: true });
  assert.deepEqual(model.dropTarget, { id: "b", after: true });
  [model] = update(model, { type: "dragLeave", id: "a" });
  assert.equal(model.dropTarget?.id, "b");
  [model] = update(model, {
    type: "dragOver",
    target: c("x", "p2"),
    after: false,
  });
  assert.equal(model.dropTarget, null);
  [model] = update(model, { type: "dragEnd" });
  assert.equal(model.dragging, null);
});

test("reorder errors and uncertain replies refetch once without retrying the command", () => {
  for (const outcome of [
    {
      kind: "rejected",
      error: { code: "invalid_request", message: "The chat was removed." },
    },
    { kind: "uncertain", reason: "connection lost" },
  ] as const) {
    const [optimistic] = move(base, p("p3"), p("p1"), false);
    assert.deepEqual(
      move(optimistic, p("p1"), p("p2"), true)[1],
      [],
      "only one reorder may be in flight",
    );
    const [failed, effects] = update(optimistic, {
      type: "reply",
      name: "reorderProjects",
      sessionId: "",
      orderGeneration: optimistic.orderGeneration,
      outcome,
    });
    assert.match(
      failed.error,
      outcome.kind === "rejected" ? /The chat was removed/ : /connection lost/,
    );
    assert.deepEqual(effects, [
      {
        type: "request",
        name: "listSessions",
        body: {},
        sessionId: "",
        orderGeneration: failed.orderGeneration,
        listRequest: 1,
      },
    ]);
    const [refreshed, more] = update(failed, {
      type: "reply",
      name: "listSessions",
      sessionId: "",
      orderGeneration: failed.orderGeneration,
      outcome: {
        kind: "replied",
        reply: { sessions: base.sessions, projects: base.projects, models: [] },
      },
    });
    assert.deepEqual(ids(refreshed), ids(base));
    assert.equal(refreshed.error, failed.error);
    assert.equal(refreshed.reorderState, null);
    assert.deepEqual(more, []);
    const [unreachable, retries] = update(failed, {
      type: "reply",
      name: "listSessions",
      sessionId: "",
      orderGeneration: failed.orderGeneration,
      outcome: { kind: "uncertain", reason: "still offline" },
    });
    assert.match(unreachable.error, /still offline/);
    assert.deepEqual(retries, []);
    assert.equal(unreachable.reorderState, null);
  }
});

test("successful reorder releases the next drag and a background list cannot release it early", () => {
  const [saving] = move(base, p("p3"), p("p1"), false);
  const [listed] = update(saving, {
    type: "reply",
    name: "listSessions",
    sessionId: "",
    outcome: {
      kind: "replied",
      reply: { projects: base.projects, sessions: base.sessions, models: [] },
    },
  });
  assert.equal(listed.reorderState, "saving");
  assert.deepEqual(ids(listed), ids(saving));
  const [saved, effects] = update(saving, {
    type: "reply",
    name: "reorderProjects",
    sessionId: "",
    orderGeneration: saving.orderGeneration,
    outcome: { kind: "replied", reply: {} },
  });
  assert.equal(saved.reorderState, "refreshing");
  assert.deepEqual(effects, [
    {
      type: "request",
      name: "listSessions",
      body: {},
      sessionId: "",
      orderGeneration: saved.orderGeneration,
      listRequest: 1,
    },
  ]);
  const [fresh] = update(saved, {
    type: "reply",
    name: "listSessions",
    sessionId: "",
    orderGeneration: saved.orderGeneration,
    outcome: {
      kind: "replied",
      reply: { projects: saved.projects, sessions: saved.sessions, models: [] },
    },
  });
  assert.equal(fresh.reorderState, null);
});

test("shortcut numbering follows reordered projects and chats while folds stay attached", () => {
  const [moved] = move(
    { ...base, collapsed: { p2: true } },
    p("p3"),
    p("p1"),
    false,
  );
  const [chatsMoved] = move(
    { ...moved, reorderState: null },
    c("c"),
    c("a"),
    false,
  );
  assert.deepEqual(
    visibleChats(chatsMoved).map((s) => s.sessionId),
    ["c", "a", "b"],
  );
  assert.deepEqual(chatsMoved.collapsed, { p2: true });
  assert.equal(update(chatsMoved, { type: "jump", number: 1 })[0].active, "c");
});

test("subagents are hidden from the sidebar and numbered shortcuts", async () => {
  const subagent: SessionInfo = {
    ...chat("sub", "p1"),
    parentSessionId: "a",
    rootSessionId: "a",
    role: "implementer",
  };
  const model: Model = { ...base, sessions: [...base.sessions, subagent] };
  assert.equal(
    visibleChats(model).some((session) => session.sessionId === "sub"),
    false,
  );
  const { dom, handle } = await mounted(model);
  const text = dom.window.document.querySelector(".sidebar")?.textContent ?? "";
  assert.doesNotMatch(text, /\bsub\b/);
  handle.stop();
  dom.window.close();
});

async function mounted(model: Model = base) {
  const dom = new JSDOM('<div id="app"></div>', { url: "http://localhost" });
  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    Node: dom.window.Node,
    Element: dom.window.Element,
    HTMLElement: dom.window.HTMLElement,
    HTMLInputElement: dom.window.HTMLInputElement,
    HTMLTextAreaElement: dom.window.HTMLTextAreaElement,
    HTMLSelectElement: dom.window.HTMLSelectElement,
    requestAnimationFrame: (fn: FrameRequestCallback) => setTimeout(fn, 0),
  });
  const handle = mountProgram({
    node: dom.window.document.getElementById("app")!,
    flags: model,
    program: { ...program, subscriptions: () => [] },
  });
  await handle.settle();
  return { dom, handle };
}
function dragEvent(dom: JSDOM, type: string, y = 0) {
  return new dom.window.MouseEvent(type, {
    bubbles: true,
    cancelable: true,
    clientY: y,
  });
}

test("native drag bindings set transfer data, show indicators and keep project buttons out", async () => {
  const { dom, handle } = await mounted();
  try {
    const doc = dom.window.document;
    const heads = [...doc.querySelectorAll<HTMLElement>(".project-head")];
    const rows = [...doc.querySelectorAll<HTMLElement>(".session")];
    assert.ok(heads.every((h) => h.draggable));
    assert.ok(rows.every((r) => r.draggable));
    const transfer = {
      effectAllowed: "",
      setData: (type: string, id: string) => {
        assert.equal(type, "text/plain");
        assert.equal(id, "p1");
      },
    };
    const start = dragEvent(dom, "dragstart");
    Object.defineProperty(start, "dataTransfer", { value: transfer });
    heads[0]!.dispatchEvent(start);
    await handle.settle();
    assert.equal(transfer.effectAllowed, "move");
    const enter = dragEvent(dom, "dragover", 1);
    heads[1]!.dispatchEvent(enter);
    await handle.settle();
    assert.ok(enter.defaultPrevented);
    assert.ok(heads[1]!.classList.contains("drop-after"));
    heads[1]!.dispatchEvent(dragEvent(dom, "dragleave"));
    await handle.settle();
    assert.ok(!heads[1]!.classList.contains("drop-after"));
    heads[0]!.dispatchEvent(dragEvent(dom, "dragend"));
    await handle.settle();
    assert.ok(!heads[0]!.classList.contains("dragging"));
    const blocked = dragEvent(dom, "dragstart");
    heads[0]!.querySelector("[data-new-chat]")!.dispatchEvent(blocked);
    await handle.settle();
    assert.ok(blocked.defaultPrevented);
    assert.ok(!heads[0]!.classList.contains("dragging"));
    rows[0]!.dispatchEvent(dragEvent(dom, "dragstart"));
    await handle.settle();
    const wrong = dragEvent(dom, "dragover", 1);
    heads[1]!.dispatchEvent(wrong);
    await handle.settle();
    assert.equal(wrong.defaultPrevented, false);
    assert.ok(!heads[0]!.classList.contains("dragging"));
    rows[2]!.dispatchEvent(dragEvent(dom, "dragover", 1));
    await handle.settle();
    assert.ok(rows[2]!.classList.contains("drop-after"));
    rows[2]!.dispatchEvent(dragEvent(dom, "drop", 1));
    await handle.settle();
    assert.equal(
      handle.effects().at(-1)?.effect.type === "request"
        ? (handle.effects().at(-1)!.effect as { name: string }).name
        : "",
      "reorderChats",
    );
    assert.equal(doc.querySelector(".session kbd")?.textContent, "1");
  } finally {
    handle.stop();
    dom.window.close();
  }
});

test("old list replies cannot undo a saved move or settle its recovery refresh", () => {
  const oldList = {
    type: "reply",
    name: "listSessions",
    sessionId: "",
    outcome: {
      kind: "replied",
      reply: { projects: base.projects, sessions: base.sessions, models: [] },
    },
  } as const;
  const [saving] = move(base, p("p3"), p("p1"), false);
  const [saved] = update(saving, {
    type: "reply",
    name: "reorderProjects",
    sessionId: "",
    orderGeneration: saving.orderGeneration,
    outcome: { kind: "replied", reply: {} },
  });
  assert.equal(
    update(saved, oldList)[0],
    saved,
    "the pre-drag response cannot overwrite the saved order",
  );
  const [failed, effects] = update(saving, {
    type: "reply",
    name: "reorderProjects",
    sessionId: "",
    orderGeneration: saving.orderGeneration,
    outcome: { kind: "uncertain", reason: "lost reply" },
  });
  assert.equal(
    update(failed, oldList)[0],
    failed,
    "the pre-drag response cannot impersonate recovery",
  );
  const current = effects[0];
  assert.ok(
    current?.type === "request" &&
      current.orderGeneration === failed.orderGeneration,
  );
  assert.ok(failed.orderGeneration > saving.orderGeneration);
  const [refreshed] = update(failed, {
    ...oldList,
    orderGeneration: failed.orderGeneration,
  });
  assert.equal(refreshed.reorderState, null);
  assert.deepEqual(ids(refreshed), ids(base));
  assert.equal(
    update(saved, { ...oldList, orderGeneration: saving.orderGeneration })[0],
    saved,
    "a request issued during the save is stale too",
  );
  const [, later] = update(saved, { type: "refresh" });
  assert.ok(
    later[0]?.type === "request" &&
      later[0].orderGeneration === saved.orderGeneration,
  );
});

test("a successful move refreshes list changes that arrived while saving", () => {
  const extra = chat("new-from-another-tab", "p1");
  let [saving] = move(base, p("p3"), p("p1"), false);
  const [changed, early] = update(saving, {
    type: "listEvent",
    event: {
      kind: "created",
      createdAt: 1,
      seq: parseSequence(1),
      sessionId: extra.sessionId,
      payload: {
        id: extra.sessionId,
        working_directory: extra.workingDirectory,
      },
    },
  });
  saving = changed;
  assert.ok(
    early[0]?.type === "request" &&
      early[0].orderGeneration === saving.orderGeneration,
  );
  assert.equal(
    update(saving, {
      type: "reply",
      name: "listSessions",
      sessionId: "",
      orderGeneration: saving.orderGeneration,
      outcome: {
        kind: "replied",
        reply: {
          projects: saving.projects,
          sessions: [...base.sessions, extra],
          models: [],
        },
      },
    })[0],
    saving,
  );
  const [refreshing, effects] = update(saving, {
    type: "reply",
    name: "reorderProjects",
    sessionId: "",
    orderGeneration: saving.orderGeneration,
    outcome: { kind: "replied", reply: {} },
  });
  assert.equal(refreshing.reorderState, "refreshing");
  // The request for the new chat is still in flight, so the refresh waits as its follow-up.
  assert.deepEqual(effects, []);
  assert.equal(refreshing.listQueued, true);
  const [settled, followUp] = update(refreshing, {
    type: "reply",
    name: "listSessions",
    sessionId: "",
    orderGeneration: saving.orderGeneration,
    listRequest: refreshing.listInFlight!,
    outcome: {
      kind: "replied",
      reply: { projects: [], sessions: [], models: [] },
    },
  } as never);
  assert.equal(settled.reorderState, "refreshing");
  assert.ok(
    followUp[0]?.type === "request" &&
      followUp[0].name === "listSessions" &&
      followUp[0].orderGeneration === refreshing.orderGeneration,
  );
  const [updated] = update(settled, {
    type: "reply",
    name: "listSessions",
    sessionId: "",
    orderGeneration: refreshing.orderGeneration,
    listRequest: settled.listRequests,
    outcome: {
      kind: "replied",
      reply: {
        projects: refreshing.projects,
        sessions: [...base.sessions, extra],
        models: [],
      },
    },
  });
  assert.equal(updated.sessions.at(-1)?.sessionId, extra.sessionId);
  assert.equal(updated.reorderState, null);
});

test("a reorder reply from before logout cannot finish a later move", () => {
  const [saving] = move(base, p("p3"), p("p1"), false);
  const [locked] = update(saving, { type: "logout" });
  const [loggedIn] = update(locked, { type: "login", token: "fixture" });
  const [loaded] = update(loggedIn, {
    type: "list",
    projects: base.projects,
    sessions: base.sessions,
    models: [],
  });
  const [newSaving] = move(loaded, p("p1"), p("p3"), true);
  assert.equal(
    update(newSaving, {
      type: "reply",
      name: "reorderProjects",
      sessionId: "",
      orderGeneration: saving.orderGeneration,
      outcome: { kind: "replied", reply: {} },
    })[0],
    newSaving,
  );
});
