import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { mountProgram } from "@pairshaped/hypertea/testing";
import {
  initial,
  program,
  shortcutTarget,
  update,
  visibleChats,
  type Model,
} from "../src/app.ts";
import type { SessionInfo } from "../src/generated/contract.ts";

const status = (archived: boolean) => ({
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
});
const chat = (
  id: string,
  projectId: string,
  archived = false,
): SessionInfo => ({
  name: id,
  sessionId: id,
  workingDirectory: "/w",
  agentSessionId: null,
  agent: "claude" as const,
  projectId,
  createdAt: 1,
  status: status(archived),
});
const project = (projectId: string) => ({
  projectId,
  name: projectId,
  path: `/p/${projectId}`,
  createdAt: 1,
});
// Order on screen: a1 (active), a2 (archived), a3 (active); project b has only an archived chat;
// project c has no chats.
const model: Model = {
  ...initial("t"),
  projects: [project("a"), project("b"), project("c")],
  sessions: [
    chat("a1", "a"),
    chat("a2", "a", true),
    chat("a3", "a"),
    chat("b1", "b", true),
  ],
  models: [
    {
      model: "sonnet",
      label: "sonnet",
      agent: "claude" as const,
      efforts: ["low"],
    },
  ],
  active: "a1",
};

async function mounted(m: Model) {
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
    flags: m,
    node: dom.window.document.querySelector<HTMLElement>("#app")!,
    program,
  });
  await handle.settle();
  return { dom, handle };
}

test("archived chats leave the numbered list, so the rest renumber", () => {
  assert.deepEqual(
    visibleChats(model).map((s) => s.sessionId),
    ["a1", "a3"],
  );
  const [jumped] = update(model, { type: "jump", number: 2 });
  assert.equal(jumped.active, "a3", "Command+2 reaches the second active chat");
  const [none] = update(model, { type: "jump", number: 3 });
  assert.equal(none.active, "a1", "there is no third numbered chat");
  assert.equal(shortcutTarget({ code: "Digit2", metaKey: true }), 2);
});

test("an archived row shows a trash button where the number was; an active row keeps its number", async () => {
  const { dom, handle } = await mounted(model);
  const rows = new Map(
    [...dom.window.document.querySelectorAll(".session")].map((r) => [
      r.textContent?.trim().split(/\s/)[0] ?? "",
      r,
    ]),
  );
  const archived = [...rows.values()].filter((r) =>
    r.classList.contains("archived"),
  );
  assert.equal(archived.length, 2);
  for (const row of archived) {
    assert.equal(row.querySelector("kbd"), null, "no number");
    assert.ok(row.querySelector("[data-delete-chat]"), "a trash button");
  }
  const active = [...rows.values()].filter(
    (r) => !r.classList.contains("archived"),
  );
  assert.equal(active.length, 2);
  assert.deepEqual(
    active.map((r) => r.querySelector("kbd")?.textContent),
    ["1", "2"],
  );
  for (const row of active)
    assert.equal(row.querySelector("[data-delete-chat]"), null);
  handle.stop();
  dom.window.close();
});

test("the trash button asks for confirmation for that chat, even when another chat is open", async () => {
  const { dom, handle } = await mounted(model);
  const trash = dom.window.document.querySelector<HTMLElement>(
    '[data-delete-chat="a2"]',
  );
  assert.ok(trash);
  trash.dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true }));
  await handle.settle();
  const effects = handle.effects().map((e) => e.effect);
  const confirm = effects.find((e) => e.type === "confirmDelete");
  assert.ok(confirm && confirm.type === "confirmDelete");
  assert.equal(confirm.sessionId, "a2");
  assert.ok(
    !effects.some((e) => e.type === "request" && e.name === "deleteSession"),
    "nothing is deleted before the confirmation",
  );
  assert.equal(
    handle.model().active,
    "a1",
    "the click does not also select the row",
  );
  handle.stop();
  dom.window.close();
});

test("only an archived chat can be deleted from the sidebar", () => {
  const [next, effects] = update(model, {
    type: "requestDeleteChat",
    sessionId: "a1",
  });
  assert.equal(next, model);
  assert.deepEqual(effects, []);
});

test("a project with no active chat is dimmed, open or closed; one with an active chat is not", async () => {
  const { dom, handle } = await mounted(model);
  const heads = new Map(
    [...dom.window.document.querySelectorAll(".project-head")].map((h) => [
      h.querySelector("strong")?.textContent ?? "",
      h,
    ]),
  );
  assert.equal(
    heads.get("a")?.classList.contains("dim"),
    false,
    "has active chats",
  );
  assert.equal(
    heads.get("b")?.classList.contains("dim"),
    true,
    "only an archived chat",
  );
  assert.equal(
    heads.get("c")?.classList.contains("dim"),
    true,
    "no chats at all",
  );
  handle.stop();
  dom.window.close();
  const closed = await mounted({ ...model, collapsed: { b: true } });
  const b = [
    ...closed.dom.window.document.querySelectorAll(".project-head"),
  ].find((h) => h.querySelector("strong")?.textContent === "b");
  assert.equal(
    b?.classList.contains("dim"),
    true,
    "collapsing does not change dimming",
  );
  const a = [
    ...closed.dom.window.document.querySelectorAll(".project-head"),
  ].find((h) => h.querySelector("strong")?.textContent === "a");
  assert.equal(a?.classList.contains("dim"), false, "collapsing does not dim");
  closed.handle.stop();
  closed.dom.window.close();
});

test("a project with no chats cannot be expanded, and one with chats still toggles", () => {
  const [stuck, effects] = update(model, {
    type: "toggleProject",
    projectId: "c",
  });
  assert.equal(stuck, model, "an empty project ignores the click");
  assert.deepEqual(effects, []);
  const [folded] = update(model, { type: "toggleProject", projectId: "a" });
  assert.equal(folded.collapsed.a, true);
});

test("an empty project is drawn collapsed and says why, even if it was left open", async () => {
  const { dom, handle } = await mounted(model);
  const title = dom.window.document.querySelector<HTMLElement>(
    '[data-toggle-project="c"]',
  );
  assert.ok(title);
  assert.equal(title.getAttribute("aria-expanded"), "false");
  assert.equal(title.getAttribute("aria-disabled"), "true");
  assert.match(title.getAttribute("title") ?? "", /no chats/i);
  const chats = dom.window.document.querySelector('[aria-label="Chats in c"]');
  assert.equal(chats, null, "no chat list is drawn for it");
  const withChats = dom.window.document.querySelector<HTMLElement>(
    '[data-toggle-project="a"]',
  );
  assert.equal(withChats?.getAttribute("aria-expanded"), "true");
  handle.stop();
  dom.window.close();
});

test("creating a chat in an empty project unlocks it and opens it", () => {
  const folded: Model = { ...model, collapsed: { c: true } };
  const [next] = update(folded, {
    type: "reply",
    name: "createSession",
    sessionId: "n1",
    outcome: { kind: "replied", reply: chat("n1", "c") } as never,
  });
  assert.equal(next.collapsed.c, undefined, "it opens to show the new chat");
  const [toggled] = update(next, { type: "toggleProject", projectId: "c" });
  assert.equal(
    toggled.collapsed.c,
    true,
    "and now it can be folded like any other",
  );
});

test("archived chats replace activity dots with a full-row trash control", async () => {
  const { dom, handle } = await mounted(model);
  const doc = dom.window.document;
  const archived = [...doc.querySelectorAll(".session.archived")];
  assert.equal(archived.length, 2);
  for (const row of archived) {
    assert.equal(row.querySelector(".activity-indicator"), null);
    assert.ok(row.querySelector(".row-delete"));
    assert.ok(row.querySelector(".session-title"));
  }
  assert.equal(
    doc.querySelector(".session:not(.archived) .activity-indicator"),
    null,
  );
  handle.stop();
  dom.window.close();
});

test("only unarchived chats mid-turn show animated activity dots", async () => {
  const sessions = model.sessions.map((s) => ({
    ...s,
    status: { ...s.status, turnActive: s.sessionId !== "a1" },
  }));
  const { dom, handle } = await mounted({ ...model, sessions });
  const dots = dom.window.document.querySelectorAll(
    ".session .activity-indicator",
  );
  assert.equal(dots.length, 1);
  assert.equal(dots[0]!.getAttribute("aria-label"), "Running");
  assert.ok(dots[0]!.classList.contains("running"));
  assert.equal(dots[0]!.querySelectorAll("span").length, 3);
  handle.stop();
  dom.window.close();
});
