import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { limitTabStops } from "../src/tab-stops.ts";

test("only the composer stays tabbable across renders, markdown, and dialogs", async () => {
  const dom = new JSDOM(
    '<main id="app"><aside class="sidebar"><nav></nav><button>Chat</button></aside><div class="messages" tabindex="0"><a href="#">Link</a><details><summary>Tool</summary></details></div><textarea id="composer"></textarea><select><option>Model</option></select><input type="file"><button>Attach</button></main>',
  );
  const doc = dom.window.document;
  const root = doc.querySelector<HTMLElement>("#app")!;
  const stop = limitTabStops(doc.body);
  const composer = doc.querySelector<HTMLTextAreaElement>("#composer")!;
  assert.equal(composer.tabIndex, 0);
  for (const node of root.querySelectorAll<HTMLElement>(
    "button, a, summary, select, input, .messages, nav",
  ))
    assert.equal(node.tabIndex, -1);
  composer.focus();
  assert.equal(doc.activeElement, composer);
  const button = doc.querySelector("button")!;
  let clicked = false;
  button.onclick = () => {
    clicked = true;
  };
  button.click();
  assert.ok(clicked);
  root.insertAdjacentHTML(
    "beforeend",
    '<dialog open><input><button tabindex="0">Close</button></dialog><div class="markdown"><a href="#">New link</a><pre>Code</pre></div>',
  );
  button.tabIndex = 0;
  await new Promise((resolve) => setTimeout(resolve, 0));
  for (const node of root.querySelectorAll<HTMLElement>(
    "button, a, input, pre",
  ))
    assert.equal(node.tabIndex, -1);
  root.outerHTML =
    '<main><button>New root</button><textarea id="composer"></textarea></main>';
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(doc.querySelector<HTMLButtonElement>("button")!.tabIndex, -1);
  assert.equal(
    doc.querySelector<HTMLTextAreaElement>("#composer")!.tabIndex,
    0,
  );
  stop();
  button.tabIndex = 0;
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(button.tabIndex, 0);
  dom.window.close();
});
