import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { mountProgram } from "@pairshaped/hypertea/testing";
import { initial, program, update, type Model } from "../src/app.ts";
import type {
  DeliveryState,
  SessionInfo,
  StoredEvent,
} from "../src/generated/contract.ts";

const status = {
  selectionPending: false,
  queueHeld: true,
  running: false,
  turnActive: false,
  stopping: false,
  queued: 0,
  dispatching: 0,
  uncertain: 2,
  rejected: 0,
  archived: false,
  model: "sonnet" as const,
  effort: null,
};
const session: SessionInfo = {
  name: "",
  sessionId: "s",
  workingDirectory: "/local/s",
  agentSessionId: null,
  agent: "claude" as const,
  projectId: "p1",
  createdAt: 1,
  status,
};
const appended = (seq: number, uuid: string, content: string): StoredEvent => ({
  sessionId: "s",
  seq,
  createdAt: 1,
  kind: "message_appended",
  payload: { uuid, role: "user", content, delivery_state: "queued" },
});
const changed = (
  seq: number,
  uuid: string,
  state: DeliveryState,
): StoredEvent => ({
  sessionId: "s",
  seq,
  createdAt: 1,
  kind: "delivery_changed",
  payload: { uuid, delivery_state: state },
});
const events = [
  appended(1, "m1", "first line\nsecond line"),
  changed(2, "m1", "uncertain"),
  appended(3, "m2", "another one"),
  changed(4, "m2", "uncertain"),
  appended(5, "m3", "this one arrived"),
  changed(6, "m3", "started"),
];
const base: Model = {
  ...initial("t"),
  sessions: [session],
  models: [
    {
      model: "sonnet",
      label: "sonnet",
      agent: "claude" as const,
      efforts: ["low"],
    },
  ],
  active: "s",
  events: { s: events },
};

async function mounted(model: Model) {
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
    flags: model,
    node: dom.window.document.querySelector<HTMLElement>("#app")!,
    program,
  });
  await handle.settle();
  return { dom, handle };
}

test("the banner lists only uncertain messages and explains the hold without promising delivery either way", async () => {
  const { dom, handle } = await mounted(base);
  const banner = dom.window.document.querySelector(".uncertain-banner");
  assert.ok(banner, "banner appears for uncertain messages");
  const items = [...banner.querySelectorAll("li")].map((li) => li.textContent);
  assert.equal(items.length, 2);
  assert.match(items[0]!, /first line/);
  assert.doesNotMatch(items[0]!, /second line/, "only the first line shows");
  assert.match(banner.textContent ?? "", /may or may not have reached/);
  assert.doesNotMatch(banner.textContent ?? "", /this one arrived/);
  handle.stop();
  dom.window.close();
});

test("no banner when nothing is uncertain", async () => {
  const { dom, handle } = await mounted({
    ...base,
    events: { s: [events[4]!, events[5]!] },
  });
  assert.equal(dom.window.document.querySelector(".uncertain-banner"), null);
  handle.stop();
  dom.window.close();
});

test("Send again puts the text in the composer, never sends it, and hides that message", () => {
  const [next, effects] = update(base, {
    type: "sendAgain",
    uuid: "m1",
    text: "first line\nsecond line",
  });
  assert.equal(next.drafts.s, "first line\nsecond line");
  assert.equal(next.dismissed.m1, true);
  assert.ok(
    !effects.some((e) => e.type === "request"),
    "nothing is sent automatically",
  );
  assert.ok(effects.some((e) => e.type === "composer"));
  assert.ok(effects.some((e) => e.type === "persistDismissed"));
});

test("Send again keeps a draft the owner already typed", () => {
  const [next] = update(
    { ...base, drafts: { s: "typed already" } },
    { type: "sendAgain", uuid: "m2", text: "another one" },
  );
  assert.equal(next.drafts.s, "typed already\n\nanother one");
});

test("Dismiss hides one message, persists, and keeps the rest", async () => {
  const [next, effects] = update(base, {
    type: "dismissUncertain",
    uuid: "m1",
  });
  assert.equal(next.dismissed.m1, true);
  assert.ok(effects.some((e) => e.type === "persistDismissed"));
  const { dom, handle } = await mounted(next);
  const items = [
    ...dom.window.document.querySelectorAll(".uncertain-banner li"),
  ];
  assert.equal(items.length, 1);
  assert.match(items[0]!.textContent ?? "", /another one/);
  handle.stop();
  dom.window.close();
});

test("a remembered dismissal survives a reload", () => {
  const model = initial("t", {}, {}, {}, null, {}, { m1: true });
  assert.equal(model.dismissed.m1, true);
});

test("the turn_started progress event adds no row to the chat", async () => {
  const { projectEvents, emptyTimeline } = await import("../src/timeline.ts");
  const rows = projectEvents(emptyTimeline(), [
    {
      sessionId: "s",
      seq: 1,
      createdAt: 1,
      kind: "agent_event",
      payload: { type: "system", subtype: "turn_started", session_id: null },
    },
  ]).rows;
  assert.deepEqual(rows, []);
});
