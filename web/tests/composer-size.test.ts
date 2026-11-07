import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { fitComposer } from "../src/composer-size.ts";

const { window } = new JSDOM("<textarea></textarea>");
const textarea = (scroll: number, border = 2) => {
  const el = window.document.querySelector("textarea")!;
  Object.defineProperty(el, "scrollHeight", {
    configurable: true,
    value: scroll,
  });
  Object.defineProperty(el, "offsetHeight", {
    configurable: true,
    value: 100 + border,
  });
  Object.defineProperty(el, "clientHeight", { configurable: true, value: 100 });
  return el;
};

test("the composer is sized to its content, borders included", () => {
  const el = textarea(60);
  fitComposer(el);
  assert.equal(el.style.height, "62px");
});

test("it measures from an unconstrained height, so it can shrink as well as grow", () => {
  const el = textarea(41);
  el.style.height = "300px";
  let measuredAt = "";
  Object.defineProperty(el, "scrollHeight", {
    configurable: true,
    get: () => {
      measuredAt = el.style.height;
      return 41;
    },
  });
  fitComposer(el);
  assert.equal(measuredAt, "auto");
  assert.equal(el.style.height, "43px");
});

test("measuring the composer does not leave the message list scrolled away from where it was", () => {
  const dom = new JSDOM('<div id="messages"></div><textarea></textarea>');
  const el = dom.window.document.querySelector("textarea")!;
  const messages = dom.window.document.querySelector<HTMLElement>("#messages")!;
  Object.defineProperty(el, "scrollHeight", { configurable: true, value: 120 });
  Object.defineProperty(el, "offsetHeight", { configurable: true, value: 102 });
  Object.defineProperty(el, "clientHeight", { configurable: true, value: 100 });
  // While the box is collapsed to measure it, the list above is taller, and the browser lowers its
  // scroll position to fit; it does not come back on its own.
  let top = 2500;
  Object.defineProperty(messages, "scrollTop", {
    configurable: true,
    get: () => (el.style.height === "auto" ? Math.min(top, 2417) : top),
    set: (value: number) => {
      top = value;
    },
  });
  let clamped = 2500;
  Object.defineProperty(el, "style", {
    configurable: true,
    value: new Proxy(el.style, {
      set(target, prop, value) {
        Reflect.set(target, prop, value);
        if (prop === "height" && value === "auto")
          clamped = Math.min(top, 2417);
        if (prop === "height" && value !== "auto") top = clamped; // the clamp sticks
        return true;
      },
    }),
  });
  fitComposer(el);
  assert.equal(top, 2500, "put back where it was");
});
