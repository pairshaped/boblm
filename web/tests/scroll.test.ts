import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import {
  attachScroll,
  historyScroll,
  scrollSubscription,
} from "../src/scroll.ts";
import { initial } from "../src/app.ts";
import type { Msg } from "../src/app.ts";

// jsdom has no layout, so the measurements are set by hand.
let resizeCallbacks: Array<() => void> = [];
class FakeResizeObserver {
  callback: () => void;
  constructor(callback: () => void) {
    this.callback = callback;
    resizeCallbacks.push(callback);
  }
  observe() {}
  disconnect() {
    resizeCallbacks = resizeCallbacks.filter((c) => c !== this.callback);
  }
}
function setup() {
  const dom = new JSDOM('<div id="messages" data-session-id="s"></div>', {
    pretendToBeVisual: true,
  });
  resizeCallbacks = [];
  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    MutationObserver: dom.window.MutationObserver,
    ResizeObserver: FakeResizeObserver,
    requestAnimationFrame: dom.window.requestAnimationFrame.bind(dom.window),
    cancelAnimationFrame: dom.window.cancelAnimationFrame.bind(dom.window),
  });
  historyScroll.anchor = null;
  const messages = dom.window.document.querySelector<HTMLElement>("#messages")!;
  const measure = (height: number, client: number, top: number) => {
    Object.defineProperty(messages, "scrollHeight", {
      configurable: true,
      value: height,
    });
    Object.defineProperty(messages, "clientHeight", {
      configurable: true,
      value: client,
    });
    Object.defineProperty(messages, "scrollTop", {
      configurable: true,
      writable: true,
      value: top,
    });
  };
  const sent: Msg[] = [];
  return { dom, messages, measure, sent, dispatch: (m: Msg) => sent.push(m) };
}
const scrolled = (dom: JSDOM, el: HTMLElement) =>
  el.dispatchEvent(new dom.window.Event("scroll"));

test("while following, the view stays at the bottom when the area shrinks, as a growing composer makes it", () => {
  const { messages, measure, dispatch } = setup();
  measure(3000, 500, 2500); // at the bottom
  const detach = attachScroll(messages, "s", dispatch);
  // The composer grows by about 83 px, so the message area is shorter and the end is hidden.
  measure(3000, 417, 2500);
  resizeCallbacks.forEach((c) => c());
  assert.equal(messages.scrollTop, 3000, "pinned to the end again");
  detach();
});

test("a stray scroll event after the composer grew does not end following", () => {
  const { dom, messages, measure, sent, dispatch } = setup();
  measure(3000, 500, 2500);
  const detach = attachScroll(messages, "s", dispatch);
  measure(3000, 417, 2500); // 83 px short of the bottom, past the 80 px margin
  resizeCallbacks.forEach((c) => c()); // the resize puts the view back at the end first
  measure(3000, 417, 2583);
  scrolled(dom, messages); // then any scroll event reads the distance
  assert.deepEqual(sent.filter((m) => m.type === "following").at(-1), {
    type: "following",
    value: true,
  });
  detach();
});

test("once the owner scrolls away, nothing moves the view, whatever arrives or resizes", () => {
  const { dom, messages, measure, sent, dispatch } = setup();
  measure(3000, 500, 2500);
  const detach = attachScroll(messages, "s", dispatch);
  measure(3000, 500, 1000); // scrolled up
  scrolled(dom, messages);
  assert.deepEqual(sent.filter((m) => m.type === "following").at(-1), {
    type: "following",
    value: false,
  });
  measure(3400, 500, 1000); // new content below
  messages.append(dom.window.document.createElement("p"));
  resizeCallbacks.forEach((c) => c());
  assert.equal(messages.scrollTop, 1000, "it stays where the owner left it");
  detach();
});

test("scrolling back to the bottom follows again, and new content then keeps it pinned", () => {
  const { dom, messages, measure, sent, dispatch } = setup();
  measure(3000, 500, 1000);
  const detach = attachScroll(messages, "s", dispatch);
  scrolled(dom, messages);
  measure(3000, 500, 2500);
  scrolled(dom, messages);
  assert.deepEqual(sent.filter((m) => m.type === "following").at(-1), {
    type: "following",
    value: true,
  });
  measure(3300, 500, 2500);
  resizeCallbacks.forEach((c) => c());
  assert.equal(messages.scrollTop, 3300);
  detach();
});

test("an image finishing loading, which makes the content taller, keeps a following view at the bottom", () => {
  const { dom, messages, measure, dispatch } = setup();
  const image = dom.window.document.createElement("img");
  messages.append(image);
  measure(3000, 500, 2500);
  const detach = attachScroll(messages, "s", dispatch);
  measure(3300, 500, 2500);
  image.dispatchEvent(new dom.window.Event("load")); // load does not bubble; it is caught on the way down
  assert.equal(messages.scrollTop, 3300);
  detach();
});

test("detaching stops all of it", () => {
  const { messages, measure, dispatch } = setup();
  measure(3000, 500, 2500);
  const detach = attachScroll(messages, "s", dispatch);
  detach();
  assert.equal(resizeCallbacks.length, 0);
});

test("a chat open from the first draw is still attached to, even if its message area is drawn a moment later", async () => {
  const { dom, measure, sent, dispatch } = setup();
  dom.window.document.querySelector("#messages")!.remove();
  const model = { ...initial("t"), active: "s" };
  const subscription = scrollSubscription(model);
  assert.ok("subscribe" in subscription);
  const stop = subscription.subscribe(dispatch, {} as never);
  // The area appears after the subscription began, as it can for a chat reopened at startup.
  await new Promise((r) => setTimeout(r, 40));
  const area = dom.window.document.createElement("div");
  area.id = "messages";
  area.dataset.sessionId = "s";
  dom.window.document.body.append(area);
  await new Promise((r) => setTimeout(r, 80));
  Object.defineProperty(area, "scrollHeight", {
    configurable: true,
    value: 3000,
  });
  Object.defineProperty(area, "clientHeight", {
    configurable: true,
    value: 500,
  });
  Object.defineProperty(area, "scrollTop", {
    configurable: true,
    writable: true,
    value: 0,
  });
  area.dispatchEvent(new dom.window.Event("scroll"));
  assert.deepEqual(sent.filter((m) => m.type === "following").at(-1), {
    type: "following",
    value: false,
  });
  stop();
  void measure;
});

test("the lock is centred on the scrollbar, whatever width this system draws it", () => {
  const { dom, messages, measure, dispatch } = setup();
  const frame = dom.window.document.createElement("div");
  messages.replaceWith(frame);
  frame.append(messages);
  measure(3000, 500, 2500);
  const width = (offset: number, client: number) => {
    Object.defineProperty(messages, "offsetWidth", {
      configurable: true,
      value: offset,
    });
    Object.defineProperty(messages, "clientWidth", {
      configurable: true,
      value: client,
    });
  };
  width(815, 800); // a 15 px scrollbar that takes up room
  const detach = attachScroll(messages, "s", dispatch);
  assert.equal(frame.style.getPropertyValue("--scrollbar-width"), "15px");
  width(811, 800); // the window moved to a screen where it is 11 px
  resizeCallbacks.forEach((c) => c());
  assert.equal(frame.style.getPropertyValue("--scrollbar-width"), "11px");
  width(800, 800); // an overlay scrollbar takes no room, so the style's own default applies
  resizeCallbacks.forEach((c) => c());
  assert.equal(frame.style.getPropertyValue("--scrollbar-width"), "");
  detach();
});

test("a short grouped-tool window does not page on restoration scroll events", () => {
  const { dom, messages, measure, sent, dispatch } = setup();
  measure(150, 600, 0);
  messages.dataset.hasOlder = "true";
  messages.dataset.hasNewer = "true";
  const detach = attachScroll(messages, "s", dispatch);
  for (let i = 0; i < 5; i++) scrolled(dom, messages);
  assert.equal(
    sent.filter((msg) => msg.type === "history").length,
    0,
    "restoring a short window must not bounce between paging edges",
  );
  messages.dispatchEvent(new dom.window.WheelEvent("wheel", { deltaY: -100 }));
  scrolled(dom, messages);
  assert.deepEqual(
    sent.filter((msg) => msg.type === "history"),
    [{ type: "history", direction: "older" }],
  );
  sent.length = 0;
  for (let i = 0; i < 5; i++) scrolled(dom, messages);
  assert.equal(sent.filter((msg) => msg.type === "history").length, 0);
  messages.dispatchEvent(new dom.window.WheelEvent("wheel", { deltaY: 100 }));
  scrolled(dom, messages);
  assert.deepEqual(
    sent.filter((msg) => msg.type === "history"),
    [{ type: "history", direction: "newer" }],
  );
  detach();
  dom.window.close();
});

test("touch and keyboard paging follow gesture direction, and expired intent cannot page", (t) => {
  t.mock.timers.enable({ apis: ["Date"], now: 0 });
  const { dom, messages, measure, sent, dispatch } = setup();
  measure(3000, 600, 1000);
  messages.dataset.hasOlder = "true";
  messages.dataset.hasNewer = "true";
  const detach = attachScroll(messages, "s", dispatch);
  messages.dispatchEvent(new dom.window.WheelEvent("wheel", { deltaY: -100 }));
  t.mock.timers.setTime(2000);
  measure(3000, 600, 0);
  scrolled(dom, messages);
  assert.equal(sent.filter((msg) => msg.type === "history").length, 0);
  const touch = (type: string, y: number) => {
    const e = new dom.window.Event(type);
    Object.defineProperty(e, "touches", { value: [{ clientY: y }] });
    messages.dispatchEvent(e);
  };
  touch("touchstart", 200);
  touch("touchmove", 300);
  scrolled(dom, messages);
  assert.deepEqual(
    sent.filter((msg) => msg.type === "history"),
    [{ type: "history", direction: "older" }],
  );
  sent.length = 0;
  measure(3000, 600, 2400);
  messages.dispatchEvent(
    new dom.window.KeyboardEvent("keydown", { key: "PageDown" }),
  );
  scrolled(dom, messages);
  assert.deepEqual(
    sent.filter((msg) => msg.type === "history"),
    [{ type: "history", direction: "newer" }],
  );
  detach();
  dom.window.close();
});
