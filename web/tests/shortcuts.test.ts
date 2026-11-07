import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { mountProgram } from "@pairshaped/hypertea/testing";
import {
  initial,
  program,
  update,
  visibleChats,
  shortcutTarget,
  type Model,
} from "../src/app.ts";
import type { ProjectInfo, SessionInfo } from "../src/generated/contract.ts";

const P1 = "20000000-0000-4000-8000-000000000001";
const P2 = "20000000-0000-4000-8000-000000000002";
const project = (id: string, name: string): ProjectInfo => ({
  projectId: id,
  name,
  path: `/work/${name}`,
  createdAt: 1,
});
const chat = (id: string, projectId: string): SessionInfo => ({
  name: id,
  sessionId: id,
  projectId,
  agent: "claude",
  workingDirectory: "/work",
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
    model: "sonnet",
    effort: null,
  },
});
const base: Model = {
  ...initial("t"),
  projects: [project(P1, "app"), project(P2, "docs")],
  sessions: [chat("a1", P1), chat("b1", P2), chat("a2", P1), chat("b2", P2)],
};

test("the numbered chats are the ones showing in the sidebar, project by project", () => {
  assert.deepEqual(
    visibleChats(base).map((s) => s.sessionId),
    ["a1", "a2", "b1", "b2"],
  );
  const folded = update(base, { type: "toggleProject", projectId: P1 })[0];
  assert.deepEqual(
    visibleChats(folded).map((s) => s.sessionId),
    ["b1", "b2"],
    "chats in a collapsed project have no number",
  );
  const [reopened, effects] = update(folded, {
    type: "toggleProject",
    projectId: P1,
  });
  assert.equal(visibleChats(reopened).length, 4);
  assert.deepEqual(effects, [{ type: "persistCollapsed", collapsed: {} }]);
});

test("jumping to a number selects that chat and puts the cursor in its composer", () => {
  const [next, effects] = update(base, { type: "jump", number: 3 });
  assert.equal(next.active, "b1");
  assert.deepEqual(effects[0], {
    type: "composer",
    sessionId: "b1",
    text: "",
  });
  assert.deepEqual(
    update(base, { type: "jump", number: 9 }),
    [base, []],
    "a number with no chat does nothing",
  );
});

test("Command plus a digit on a Mac, Alt plus a digit elsewhere; nothing else counts", () => {
  const key = (init: Partial<KeyboardEventInit> & { code: string }) =>
    shortcutTarget(init);
  assert.equal(key({ code: "Digit1", metaKey: true }), 1);
  assert.equal(key({ code: "Digit9", altKey: true }), 9);
  assert.equal(key({ code: "Digit0", metaKey: true }), null, "0 is not a chat");
  assert.equal(key({ code: "Digit1" }), null, "a bare digit is typing");
  assert.equal(key({ code: "Digit1", metaKey: true, shiftKey: true }), null);
  assert.equal(
    key({ code: "Digit1", ctrlKey: true }),
    null,
    "Ctrl switches browser tabs",
  );
  assert.equal(key({ code: "KeyA", metaKey: true }), null);
  assert.equal(
    key({ code: "Digit2", altKey: true, repeat: true }),
    null,
    "holding the key does not repeat",
  );
});

test("a new chat in a folded project opens the project so it shows", () => {
  const folded = update(base, { type: "toggleProject", projectId: P1 })[0];
  const [next] = update(folded, {
    type: "reply",
    name: "createSession",
    sessionId: "a3",
    outcome: { kind: "replied", reply: chat("a3", P1) },
  });
  assert.ok(visibleChats(next).some((s) => s.sessionId === "a3"));
  assert.equal(next.active, "a3");
});

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

test("each showing chat carries its number, up to nine, and a folded project hides its chats", async () => {
  const { dom, handle } = await mounted(base);
  const numbers = [...dom.window.document.querySelectorAll(".session")].map(
    (el) => el.querySelector(".shortcut")?.textContent,
  );
  assert.deepEqual(numbers, ["1", "2", "3", "4"]);
  dom.window.document
    .querySelector<HTMLButtonElement>(`[data-toggle-project="${P1}"]`)!
    .click();
  await handle.settle();
  const after = [...dom.window.document.querySelectorAll(".session")].map(
    (el) => el.querySelector(".shortcut")?.textContent,
  );
  assert.deepEqual(after, ["1", "2"]);
  assert.equal(
    dom.window.document
      .querySelector(`[data-toggle-project="${P1}"]`)
      ?.getAttribute("aria-expanded"),
    "false",
  );
  handle.stop();
  dom.window.close();
});

test("more than nine chats: only the first nine have a number", () => {
  const many: Model = {
    ...base,
    sessions: Array.from({ length: 11 }, (_, i) => chat(`c${i}`, P1)),
  };
  assert.equal(visibleChats(many).length, 11);
  assert.deepEqual(update(many, { type: "jump", number: 9 })[0].active, "c8");
});
