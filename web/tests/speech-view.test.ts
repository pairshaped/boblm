import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { mountProgram } from "@pairshaped/hypertea/testing";
import { initial, program, update, type Model } from "../src/app.ts";
import type { SessionInfo, StoredEvent } from "../src/generated/contract.ts";

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
const user = (seq: number, content: string): StoredEvent => ({
  sessionId: "s1",
  seq,
  createdAt: 1_700_000_000_000,
  kind: "message_appended",
  payload: {
    uuid: `u${seq}`,
    role: "user",
    content,
    delivery_state: "started",
  },
});
const agent = (seq: number, text: string): StoredEvent => ({
  sessionId: "s1",
  seq,
  createdAt: 1_700_000_000_000,
  kind: "agent_event",
  payload: {
    type: "assistant",
    message: { id: `a${seq}`, content: [{ type: "text", text }] },
  },
});
const base: Model = {
  ...initial("t"),
  projects: [{ projectId: "p", name: "p", path: "/p", createdAt: 1 }],
  sessions: [chat("s1"), chat("s2")],
  models: [
    {
      model: "sonnet",
      label: "sonnet",
      agent: "claude" as const,
      efforts: ["low"],
    },
  ],
  active: "s1",
  events: {
    s1: [
      user(1, "please explain"),
      agent(2, "Here is **the answer** with `code`."),
      agent(3, "```sh\nls\n```"),
    ],
    s2: [],
  },
};

async function mounted(m: Model, supported = true) {
  const dom = new JSDOM('<div id="app"></div>', {
    pretendToBeVisual: true,
    url: "http://localhost",
  });
  if (supported)
    Object.assign(dom.window, {
      speechSynthesis: { speak() {}, cancel() {} },
      SpeechSynthesisUtterance: class {},
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

test("only agent messages with something to read get a read-aloud button", async () => {
  const { dom, handle } = await mounted(base);
  const doc = dom.window.document;
  const buttons = doc.querySelectorAll("button[aria-label='Read aloud']");
  assert.equal(buttons.length, 1, "one agent message is readable");
  assert.ok(buttons[0]!.closest(".message.assistant"));
  assert.equal(
    doc.querySelector(".message.user button[aria-label='Read aloud']"),
    null,
  );
  handle.stop();
  dom.window.close();
});

test("no button where the browser cannot speak", async () => {
  const { dom, handle } = await mounted(base, false);
  assert.equal(
    dom.window.document.querySelector("button[aria-label='Read aloud']"),
    null,
  );
  handle.stop();
  dom.window.close();
});

test("clicking it reads the prose, not the code, and the button becomes Stop reading", async () => {
  const { dom, handle } = await mounted(base);
  dom.window.document
    .querySelector<HTMLElement>("button[aria-label='Read aloud']")!
    .dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true }));
  await handle.settle();
  const effect = handle
    .effects()
    .map((e) => e.effect)
    .find((e) => e.type === "speak");
  assert.ok(effect && effect.type === "speak");
  assert.equal(effect.text, "Here is the answer with.");
  const stop = dom.window.document.querySelector(
    "button[aria-label='Stop reading']",
  );
  assert.ok(stop);
  assert.equal(stop.getAttribute("aria-pressed"), "true");
  assert.equal(handle.model().speaking, effect.key);
  handle.stop();
  dom.window.close();
});

test("starting, finishing, stopping and failing update the state", () => {
  const [reading, effects] = update(base, {
    type: "readAloud",
    key: "a2",
    text: "hello",
  });
  assert.equal(reading.speaking, "a2");
  assert.deepEqual(effects, [{ type: "speak", key: "a2", text: "hello" }]);
  const [done] = update(reading, {
    type: "speechEnded",
    key: "a2",
    failed: false,
  });
  assert.equal(done.speaking, null);
  assert.equal(done.speechFailed, null);
  const [failed] = update(reading, {
    type: "speechEnded",
    key: "a2",
    failed: true,
  });
  assert.equal(failed.speaking, null);
  assert.equal(failed.speechFailed, "a2");
  const [stopped, stopEffects] = update(reading, { type: "stopReading" });
  assert.equal(stopped.speaking, null);
  assert.deepEqual(stopEffects, [{ type: "stopSpeech" }]);
  const [late] = update(
    { ...reading, speaking: "a9" },
    { type: "speechEnded", key: "a2", failed: true },
  );
  assert.equal(
    late.speaking,
    "a9",
    "an ending reading does not cancel a newer one",
  );
  assert.equal(late.speechFailed, null);
});

test("a failure says so beside the button in plain words", async () => {
  const { dom, handle } = await mounted({ ...base, speechFailed: "a2" });
  const status = dom.window.document.querySelector(
    ".message-actions [role='status']",
  );
  assert.match(status?.textContent ?? "", /could not read this aloud/i);
  handle.stop();
  dom.window.close();
});

test("switching chats stops the reading", () => {
  const [switched, effects] = update(
    { ...base, speaking: "a2" },
    { type: "select", id: "s2" },
  );
  assert.equal(switched.speaking, null);
  assert.ok(effects.some((e) => e.type === "stopSpeech"));
});
