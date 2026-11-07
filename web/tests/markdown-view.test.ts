import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { mountProgram } from "@pairshaped/hypertea/testing";
import { initial, program, update, type Model } from "../src/app.ts";
import type { SessionInfo, StoredEvent } from "../src/generated/contract.ts";

const session: SessionInfo = {
  name: "",
  sessionId: "s",
  workingDirectory: "/local/s",
  agentSessionId: null,
  agent: "claude" as const,
  projectId: "p1",
  createdAt: 1,
  status: {
    selectionPending: false,
    queueHeld: false,
    running: true,
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
};
const user = (seq: number, content: string): StoredEvent => ({
  sessionId: "s",
  seq,
  createdAt: 1,
  kind: "message_appended",
  payload: {
    uuid: `u${seq}`,
    role: "user",
    content,
    delivery_state: "started",
  },
});
const agent = (seq: number, text: string): StoredEvent => ({
  sessionId: "s",
  seq,
  createdAt: 1,
  kind: "agent_event",
  payload: {
    type: "assistant",
    message: { id: `a${seq}`, content: [{ type: "text", text }] },
  },
});
const model: Model = {
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
  events: {
    s: [
      user(1, "**not bold** | a | b |"),
      agent(2, "| a | b |\n|---|---|\n| 1 | 2 |\n\n```sh\nls -la\n```"),
    ],
  },
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

test("agent replies render as markdown and the owner's own messages stay plain text", async () => {
  const { dom, handle } = await mounted(model);
  const doc = dom.window.document;
  assert.ok(doc.querySelector(".message.assistant .markdown table"), "table");
  assert.ok(
    doc.querySelector(".message.assistant .markdown .code-block"),
    "code",
  );
  const owner = doc.querySelector(".message.user");
  assert.equal(owner?.querySelector(".markdown"), null);
  assert.equal(
    owner?.querySelector("pre")?.textContent,
    "**not bold** | a | b |",
  );
  handle.stop();
  dom.window.close();
});

test("clicking a code block's copy button copies only that block's text", async () => {
  const { dom, handle } = await mounted(model);
  const button =
    dom.window.document.querySelector<HTMLElement>("button.copy-code");
  assert.ok(button);
  button.dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true }));
  await handle.settle();
  const copy = handle
    .effects()
    .map((e) => e.effect)
    .find((e) => e.type === "copy");
  assert.ok(copy && copy.type === "copy");
  assert.equal(copy.text, "ls -la");
  handle.stop();
  dom.window.close();
});

test("a copy request becomes a copy effect and changes nothing else", () => {
  const [next, effects] = update(model, { type: "copyText", text: "hi" });
  assert.equal(next, model);
  assert.deepEqual(effects, [{ type: "copy", text: "hi" }]);
});
