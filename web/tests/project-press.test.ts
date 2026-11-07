import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { listenForProjectRename } from "../src/project-press.ts";
import { limits } from "../src/generated/contract.ts";

test("project holds rename once and suppress collapse, while clicks and drags stay normal", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const dom = new JSDOM('<button data-toggle-project="p">Project</button>');
  const doc = dom.window.document;
  const button = doc.querySelector("button")!;
  const names: string[] = [];
  let clicks = 0;
  button.addEventListener("click", () => clicks++);
  const stop = listenForProjectRename(doc, (id) => names.push(id));
  const pointer = (type: string, x = 0) => {
    const event = new dom.window.MouseEvent(type, {
      bubbles: true,
      clientX: x,
      button: 0,
    });
    Object.defineProperties(event, {
      isPrimary: { value: true },
      pointerId: { value: 1 },
    });
    button.dispatchEvent(event);
  };
  pointer("pointerdown");
  pointer("pointerup");
  button.click();
  t.mock.timers.tick(limits.longPressMs);
  assert.equal(clicks, 1);
  assert.deepEqual(names, []);
  pointer("pointerdown");
  t.mock.timers.tick(limits.longPressMs);
  pointer("pointerup");
  button.click();
  assert.equal(clicks, 1);
  assert.deepEqual(names, ["p"]);
  for (const cancel of [
    "pointermove",
    "pointercancel",
    "dragstart",
    "scroll",
  ]) {
    pointer("pointerdown");
    pointer(cancel, limits.longPressMovePx + 1);
    t.mock.timers.tick(limits.longPressMs);
    assert.equal(names.length, 1);
  }
  button.dispatchEvent(
    new dom.window.KeyboardEvent("keydown", { key: "F2", bubbles: true }),
  );
  assert.deepEqual(names, ["p", "p"]);
  pointer("pointerdown");
  stop();
  t.mock.timers.tick(limits.longPressMs);
  assert.equal(names.length, 2);
  dom.window.close();
});
