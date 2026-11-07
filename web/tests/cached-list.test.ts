import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { mountProgram } from "@pairshaped/hypertea/testing";
import { initial, program, update, type Model } from "../src/app.ts";
import type { SessionInfo } from "../src/generated/contract.ts";

const chat = (id: string, name: string): SessionInfo => ({
  name,
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
const list = {
  sessions: [chat("s1", "Planning notes"), chat("s2", "Fix the login bug")],
  projects: [{ projectId: "p", name: "Website", path: "/p", createdAt: 1 }],
  models: [
    {
      model: "sonnet",
      label: "sonnet",
      agent: "claude" as const,
      efforts: ["low"],
    },
  ],
};
const cached = (restore: string | null = null): Model =>
  initial("t", {}, {}, {}, null, {}, {}, false, restore, list);

test("a remembered chat list is on screen at once, before the server has answered", async () => {
  const dom = new JSDOM('<div id="app"></div>', {
    pretendToBeVisual: true,
    url: "http://localhost",
  });
  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    Node: dom.window.Node,
    Element: dom.window.Element,
    HTMLElement: dom.window.HTMLElement,
    HTMLInputElement: dom.window.HTMLInputElement,
    HTMLTextAreaElement: dom.window.HTMLTextAreaElement,
    HTMLSelectElement: dom.window.HTMLSelectElement,
    requestAnimationFrame: dom.window.requestAnimationFrame.bind(dom.window),
    cancelAnimationFrame: dom.window.cancelAnimationFrame.bind(dom.window),
  });
  const handle = mountProgram({
    flags: cached(),
    node: dom.window.document.querySelector<HTMLElement>("#app")!,
    program,
  });
  await handle.settle();
  const text = dom.window.document.querySelector(".sidebar")?.textContent ?? "";
  assert.match(text, /Website/);
  assert.match(text, /Planning notes/);
  assert.match(text, /Fix the login bug/);
  handle.stop();
  dom.window.close();
});

test("with no cached list the sidebar starts empty, as before", () => {
  const model = initial("t");
  assert.deepEqual(
    [model.sessions, model.projects, model.models],
    [[], [], []],
  );
});

test("the server's answer replaces the remembered list and is remembered in turn", () => {
  const fresh = { ...list, sessions: [chat("s1", "Renamed since")] };
  const [next, effects] = update(cached(), {
    type: "reply",
    name: "listSessions",
    sessionId: "",
    outcome: { kind: "replied", reply: fresh } as never,
  });
  assert.deepEqual(
    next.sessions.map((s) => s.name),
    ["Renamed since"],
  );
  const saved = effects.find((e) => e.type === "persistList");
  assert.ok(saved && saved.type === "persistList");
  assert.deepEqual(
    saved.list.sessions.map((s) => s.name),
    ["Renamed since"],
  );
});

test("the last chat reopens straight away from the remembered list, and the fresh list is still requested", () => {
  const [model, effects] = program.init(cached("s2")) as [
    Model,
    Array<{ type: string; name?: string; sessionId?: string }>,
  ];
  assert.equal(model.active, "s2");
  assert.equal(model.restoreChat, null);
  assert.ok(
    effects.some((e) => e.type === "request" && e.name === "listSessions"),
  );
  assert.ok(effects.some((e) => e.type === "composer" && e.sessionId === "s2"));
});

test("a remembered last chat that is not in the remembered list waits for the server", () => {
  const [model] = program.init(cached("missing")) as [Model, unknown[]];
  assert.equal(model.active, null);
  assert.equal(model.restoreChat, "missing");
});

test("locked out, nothing is requested and nothing is opened", () => {
  const [model, effects] = program.init({ ...cached("s2"), token: "" }) as [
    Model,
    unknown[],
  ];
  assert.equal(model.active, null);
  assert.deepEqual(effects, []);
});

test("list snapshots skip historical working states on reconnect", () => {
  const [snapshot] = update(cached(), {
    type: "reply",
    name: "listSessions",
    sessionId: "",
    outcome: { kind: "replied", reply: { ...list, cursor: 100000 } } as never,
  });
  assert.equal(snapshot.listCursor, 100000);
  const [replayed, effects] = update(snapshot, {
    type: "listEvent",
    event: {
      seq: 1,
      sessionId: "s1",
      kind: "state_summary",
      createdAt: 1,
      payload: { ...list.sessions[0]!.status, turnActive: true },
    } as never,
  });
  assert.equal(replayed.sessions[0]!.status.turnActive, false);
  assert.deepEqual(effects, []);
  assert.equal(replayed, snapshot);
});
