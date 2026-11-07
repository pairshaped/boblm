import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { mountProgram } from "@pairshaped/hypertea/testing";
import {
  initial,
  program,
  update,
  type HistoryState,
  type Model,
} from "../src/app.ts";
import { emptyTimeline } from "../src/timeline.ts";
import type { SessionInfo } from "../src/generated/contract.ts";

const chat: SessionInfo = {
  name: "c",
  sessionId: "s",
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
};
const history = (following: boolean): HistoryState => ({
  timeline: emptyTimeline(),
  loaded: true,
  loading: false,
  hasOlder: false,
  hasNewer: false,
  following,
  generation: 0,
  direction: "latest",
  error: "",
});
const base: Model = {
  ...initial("t"),
  projects: [{ projectId: "p", name: "p", path: "/p", createdAt: 1 }],
  sessions: [chat],
  models: [
    {
      model: "sonnet",
      label: "sonnet",
      agent: "claude" as const,
      efforts: ["low"],
    },
  ],
  active: "s",
  history: { s: history(true) },
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

test("a closed lock shows only while the chat follows the newest messages", async () => {
  const { dom, handle } = await mounted(base);
  const lock = dom.window.document.querySelector(
    ".messages-frame .scroll-lock",
  );
  assert.ok(lock);
  assert.equal(lock.getAttribute("data-state"), "following");
  assert.match(
    lock.getAttribute("aria-label") ?? "",
    /following new messages/i,
  );
  assert.ok(lock.querySelector("svg"));
  handle.stop();
  dom.window.close();
});

test("nothing shows once the owner has scrolled away from the bottom", async () => {
  const { dom, handle } = await mounted({
    ...base,
    history: { s: history(false) },
  });
  assert.equal(dom.window.document.querySelector(".scroll-lock"), null);
  handle.stop();
  dom.window.close();
});

test("the lock is only a sign: not a button, not focusable, and clicks go through it", async () => {
  const { dom, handle } = await mounted(base);
  const lock = dom.window.document.querySelector(".scroll-lock");
  assert.ok(lock);
  assert.equal(lock.closest("button"), null);
  assert.notEqual(lock.tagName, "BUTTON");
  assert.equal(lock.getAttribute("tabindex"), null);
  assert.equal(lock.getAttribute("role"), "img");
  handle.stop();
  dom.window.close();
});

test("no lock where no chat is open", async () => {
  const { dom, handle } = await mounted({ ...base, active: null });
  assert.equal(dom.window.document.querySelector(".scroll-lock"), null);
  handle.stop();
  dom.window.close();
});

test("scrolling away and back flips the sign", () => {
  const [away] = update(base, { type: "following", value: false });
  assert.equal(away.history.s?.following, false);
  const [back] = update(away, { type: "following", value: true });
  assert.equal(back.history.s?.following, true);
});

test("typing, dictating and sending never move a view the owner has scrolled away from", () => {
  // The real use: scroll back to the first of many questions, answer it, scroll down, answer the next.
  const away: Model = {
    ...base,
    history: { s: history(false) },
    drafts: { s: "" },
  };
  const [typed, typingEffects] = update(away, {
    type: "draft",
    text: "My answer to the first question",
  });
  assert.equal(typed.history.s?.following, false, "still not following");
  assert.ok(
    !typingEffects.some((e) => e.type === "bottom"),
    "typing does not scroll",
  );
  const [sending, sendEffects] = update(typed, {
    type: "send",
    id: "00000000-0000-4000-8000-000000000001",
  });
  assert.equal(sending.history.s?.following, false, "sending does not re-lock");
  assert.ok(
    !sendEffects.some((e) => e.type === "bottom"),
    "sending does not jump to the bottom",
  );
  const [sent, sentEffects] = update(sending, {
    type: "reply",
    name: "sendMessage",
    sessionId: "s",
    outcome: { kind: "replied", reply: { status: chat.status } } as never,
  });
  assert.equal(
    sent.history.s?.following,
    false,
    "the reply to a send does not re-lock either",
  );
  assert.ok(!sentEffects.some((e) => e.type === "bottom"));
});

test("only the owner's own scrolling or the jump button locks the view to the bottom", () => {
  const away: Model = { ...base, history: { s: history(false) } };
  const [jumped, effects] = update(away, { type: "bottom" });
  assert.ok(
    effects.some((e) => e.type === "bottom"),
    "the jump button scrolls down",
  );
  void jumped;
});
