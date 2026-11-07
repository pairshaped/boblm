import { test, type TestContext } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { initial, type Msg } from "../src/app.ts";
import { limits } from "../src/generated/contract.ts";
import { historyResumeSubscription } from "../src/history-resume.ts";

function setup(t: TestContext, initiallyHidden = false) {
  t.mock.timers.enable({ apis: ["Date", "setTimeout"], now: 0 });
  const dom = new JSDOM('<div id="messages" data-session-id="s"></div>', {
    pretendToBeVisual: true,
  });
  const saved = new Map(
    ["document", "window", "HTMLElement"].map((key) => [
      key,
      Object.getOwnPropertyDescriptor(globalThis, key),
    ]),
  );
  Object.assign(globalThis, {
    document: dom.window.document,
    window: dom.window,
    HTMLElement: dom.window.HTMLElement,
  });
  let hidden = initiallyHidden;
  Object.defineProperty(dom.window.document, "visibilityState", {
    get: () => (hidden ? "hidden" : "visible"),
  });
  const messages: Msg[] = [];
  const stop = historyResumeSubscription({
    ...initial("test"),
    active: "s",
  }).subscribe((msg) => messages.push(msg), {} as never);
  t.after(() => {
    if (typeof stop === "function") stop();
    dom.window.close();
    for (const [key, descriptor] of saved) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else Reflect.deleteProperty(globalThis, key);
    }
  });
  return {
    dom,
    messages,
    stop,
    visibility(value: boolean) {
      hidden = value;
      dom.window.document.dispatchEvent(
        new dom.window.Event("visibilitychange"),
      );
    },
  };
}

test("a stationary visible chat resets at fifteen minutes; actual scrolling restarts the timer", (t) => {
  const { dom, messages } = setup(t);
  const list = dom.window.document.querySelector<HTMLElement>("#messages")!;
  t.mock.timers.tick(limits.historyResumeLatestMs - 1);
  assert.equal(messages.length, 0);
  list.scrollTop = 100;
  list.dispatchEvent(new dom.window.Event("scroll"));
  t.mock.timers.tick(limits.historyResumeLatestMs - 1);
  assert.equal(messages.length, 0);
  // An event without a position change must not postpone the reset.
  list.dispatchEvent(new dom.window.Event("scroll"));
  t.mock.timers.tick(1);
  assert.deepEqual(messages, [
    {
      type: "visibility",
      hidden: false,
      now: 2 * limits.historyResumeLatestMs - 1,
      idle: true,
    },
  ]);
});

test("hidden chats wait until visibility or focus returns, then reset once", (t) => {
  const { dom, messages, visibility } = setup(t);
  visibility(true);
  t.mock.timers.tick(limits.historyResumeLatestMs + 1);
  assert.equal(messages.length, 1);
  visibility(false);
  dom.window.dispatchEvent(new dom.window.Event("focus"));
  assert.equal(
    messages.filter((m) => m.type === "visibility" && m.idle).length,
    1,
  );
});

test("subscription cleanup removes idle timers and focus listeners", (t) => {
  const { dom, messages, stop } = setup(t);
  if (typeof stop === "function") stop();
  t.mock.timers.tick(limits.historyResumeLatestMs);
  dom.window.dispatchEvent(new dom.window.Event("focus"));
  assert.equal(messages.length, 0);
});

test("initially hidden subscription waits until setup finishes before dispatching", async (t) => {
  const { messages } = setup(t, true);
  assert.equal(
    messages.length,
    0,
    "synchronous dispatch would re-enter subscription installation",
  );
  await Promise.resolve();
  assert.equal(messages.length, 1);
  assert.equal(messages[0]?.type, "visibility");
});

test("cleanup cancels a queued hidden-page notification", async (t) => {
  const { messages, stop } = setup(t, true);
  if (typeof stop === "function") stop();
  await Promise.resolve();
  assert.equal(messages.length, 0);
});
