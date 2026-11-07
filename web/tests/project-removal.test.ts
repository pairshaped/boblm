import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { mountProgram } from "@pairshaped/hypertea/testing";
import { initial, program, update, type Model } from "../src/app.ts";
import type { SessionInfo } from "../src/generated/contract.ts";

const chat = (
  id: string,
  projectId: string,
  archived: boolean,
): SessionInfo => ({
  name: id,
  sessionId: id,
  workingDirectory: "/w",
  agentSessionId: null,
  agent: "claude" as const,
  projectId,
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
const project = (projectId: string) => ({
  projectId,
  name: projectId,
  path: `/p/${projectId}`,
  createdAt: 1,
});
// a has an active chat; b has two archived chats; c is empty.
const model: Model = {
  ...initial("t"),
  projects: [project("a"), project("b"), project("c")],
  sessions: [
    chat("a1", "a", false),
    chat("b1", "b", true),
    chat("b2", "b", true),
  ],
  models: [
    {
      model: "sonnet",
      label: "sonnet",
      agent: "claude" as const,
      efforts: ["low"],
    },
  ],
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

test("Remove project appears when a project has no active chat, and never while one is active", async () => {
  const { dom, handle } = await mounted(model);
  const has = (id: string) =>
    !!dom.window.document.querySelector(`[data-remove-project="${id}"]`);
  assert.equal(has("a"), false, "an active chat keeps the project");
  assert.equal(has("b"), true, "only archived chats");
  assert.equal(has("c"), true, "no chats at all");
  handle.stop();
  dom.window.close();
});

test("clicking Remove project asks first, saying what goes with it, and removes nothing yet", async () => {
  const { dom, handle } = await mounted(model);
  dom.window.document
    .querySelector<HTMLElement>('[data-remove-project="b"]')!
    .dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true }));
  await handle.settle();
  const effects = handle.effects().map((e) => e.effect);
  const confirm = effects.find((e) => e.type === "confirmRemoveProject");
  assert.ok(confirm && confirm.type === "confirmRemoveProject");
  assert.deepEqual(
    [confirm.projectId, confirm.name, confirm.chats],
    ["b", "b", 2],
  );
  assert.ok(
    !effects.some((e) => e.type === "request" && e.name === "removeProject"),
    "no removal before the owner agrees",
  );
  handle.stop();
  dom.window.close();
});

test("a project with an active chat cannot even be asked about", () => {
  const [next, effects] = update(model, {
    type: "requestRemoveProject",
    projectId: "a",
  });
  assert.equal(next, model);
  assert.deepEqual(effects, []);
});

test("once the server confirms, the project and its chats leave the sidebar together", () => {
  const [next] = update(model, {
    type: "reply",
    name: "removeProject",
    sessionId: "b",
    outcome: { kind: "replied", reply: {} } as never,
  });
  assert.deepEqual(
    next.projects.map((p) => p.projectId),
    ["a", "c"],
  );
  assert.deepEqual(
    next.sessions.map((s) => s.sessionId),
    ["a1"],
  );
});
