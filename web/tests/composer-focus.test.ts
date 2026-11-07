import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { listenForComposerFocus } from "../src/composer-focus.ts";

test("chat background focuses the composer but selection and controls keep focus", () => {
  const dom = new JSDOM(
    '<aside>Sidebar</aside><section class="chat"><p>Message text</p><button>Action</button><input><details><summary>Tool</summary></details><textarea id="composer">Draft</textarea></section>',
  );
  const { document, MouseEvent, KeyboardEvent } = dom.window;
  const stop = listenForComposerFocus(document);
  const composer = document.querySelector("textarea")!;
  const message = document.querySelector("p")!;
  const click = (element: Element) =>
    element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  click(message);
  assert.equal(document.activeElement, composer);
  assert.equal(composer.value, "Draft");
  composer.blur();
  const range = document.createRange();
  range.selectNodeContents(message);
  document.getSelection()!.addRange(range);
  click(message);
  assert.notEqual(document.activeElement, composer);
  document.getSelection()!.removeAllRanges();
  for (const selector of ["button", "input", "summary", "aside"]) {
    click(document.querySelector(selector)!);
    assert.notEqual(document.activeElement, composer);
  }
  message.dispatchEvent(
    new KeyboardEvent("keydown", { key: "c", ctrlKey: true, bubbles: true }),
  );
  assert.notEqual(document.activeElement, composer);
  message.dispatchEvent(
    new KeyboardEvent("keydown", { key: "x", bubbles: true }),
  );
  assert.equal(document.activeElement, composer);
  composer.blur();
  composer.readOnly = true;
  click(message);
  assert.notEqual(document.activeElement, composer);
  composer.readOnly = false;
  stop();
  click(message);
  assert.notEqual(document.activeElement, composer);
  dom.window.close();
});

test("Escape uses the enabled Stop button and respects dialogs, IME, and sidebar focus", () => {
  const dom = new JSDOM(
    '<aside tabindex="0">Sidebar</aside><section class="chat"><div class="compose"><textarea id="composer"></textarea><button class="stop">Stop</button></div></section>',
  );
  const doc = dom.window.document;
  const stop = listenForComposerFocus(doc);
  const button = doc.querySelector<HTMLButtonElement>("button")!;
  const textarea = doc.querySelector("textarea")!;
  let clicks = 0;
  button.addEventListener("click", () => clicks++);
  const escape = (target: Element, options = {}) =>
    target.dispatchEvent(
      new dom.window.KeyboardEvent("keydown", {
        key: "Escape",
        bubbles: true,
        cancelable: true,
        ...options,
      }),
    );
  escape(textarea);
  assert.equal(clicks, 1);
  button.disabled = true;
  escape(textarea);
  button.disabled = false;
  escape(textarea, { isComposing: true });
  escape(textarea, { repeat: true });
  escape(textarea, { ctrlKey: true });
  escape(doc.querySelector("aside")!);
  const dialog = doc.createElement("div");
  dialog.setAttribute("role", "dialog");
  doc.body.append(dialog);
  escape(textarea);
  assert.equal(clicks, 1);
  dialog.remove();
  escape(doc.querySelector(".chat")!);
  assert.equal(clicks, 2);
  stop();
  dom.window.close();
});
