import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { mountProgram } from "@pairshaped/hypertea/testing";
import { initial, program, update, type Model } from "../src/app.ts";
import { closesLightbox, lightboxKey } from "../src/lightbox.ts";
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
const image = (id: string, name = "shot.png") => ({
  id,
  name,
  contentType: "image/png",
  size: 2048,
});
const shown = (
  seq: number,
  caption: string,
  ...images: ReturnType<typeof image>[]
): StoredEvent => ({
  sessionId: "s1",
  seq,
  createdAt: 1_700_000_000_000 + seq * 1000,
  kind: "message_appended",
  payload: {
    uuid: `m${seq}`,
    role: "assistant",
    content: caption,
    delivery_state: "started",
    attachments: images,
  },
});
const owner = (seq: number): StoredEvent => ({
  sessionId: "s1",
  seq,
  createdAt: 1_700_000_000_000 + seq * 1000,
  kind: "message_appended",
  payload: {
    uuid: `u${seq}`,
    role: "user",
    content: "here is my file",
    delivery_state: "started",
    attachments: [image("owner-file", "mine.png")],
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
      shown(1, "Home page after the change", image("img-a")),
      shown(2, "", image("img-b", "final-result.png")),
      owner(3),
    ],
    s2: [],
  },
  thumbs: { "img-a": "blob:a", "owner-file": "blob:owner" },
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

test("an agent's image is a labelled, collapsible section, open by default, with a thumbnail", async () => {
  const { dom, handle } = await mounted(base);
  const sections = [
    ...dom.window.document.querySelectorAll("details.agent-image"),
  ];
  assert.equal(sections.length, 2);
  const first = sections[0]!;
  assert.equal((first as HTMLDetailsElement).open, true);
  assert.equal(
    first.querySelector("summary")?.textContent,
    "Image: Home page after the change",
  );
  const picture = first.querySelector("img");
  assert.equal(picture?.getAttribute("src"), "blob:a");
  assert.equal(picture?.getAttribute("alt"), "Home page after the change");
  assert.ok(
    first.querySelector("button[data-image-open='img-a']"),
    "the thumbnail is the control that opens it",
  );
  handle.stop();
  dom.window.close();
});

test("without a caption the label is the file name, and an image still on its way says so", async () => {
  const { dom, handle } = await mounted(base);
  const second = dom.window.document.querySelectorAll(
    "details.agent-image",
  )[1]!;
  assert.equal(
    second.querySelector("summary")?.textContent,
    "Image: final-result.png",
  );
  assert.equal(
    second.querySelector("img"),
    null,
    "no thumbnail has arrived yet",
  );
  assert.match(second.textContent ?? "", /Loading image/);
  handle.stop();
  dom.window.close();
});

test("the caption is the label, not repeated as a message, and the section is not dimmed like a tool summary", async () => {
  const { dom, handle } = await mounted(base);
  const row = dom.window.document.querySelectorAll(".message.assistant")[0]!;
  assert.equal(row.querySelector(".markdown"), null);
  assert.equal(
    row.querySelector(".tool-details"),
    null,
    "it does not borrow the faded tool styling",
  );
  assert.equal(row.querySelector(".message-actions"), null);
  handle.stop();
  dom.window.close();
});

test("the owner's own attachments keep their chips", async () => {
  const { dom, handle } = await mounted(base);
  const mine = dom.window.document.querySelector(".message.user");
  assert.ok(mine?.querySelector("li.chip[data-attachment-id='owner-file']"));
  assert.equal(mine?.querySelector("details.agent-image"), null);
  handle.stop();
  dom.window.close();
});

test("tapping a thumbnail opens it full screen, with a close button, and closing returns focus to it", async () => {
  const { dom, handle } = await mounted(base);
  const doc = dom.window.document;
  assert.equal(doc.querySelector(".lightbox"), null);
  doc
    .querySelector<HTMLElement>("button[data-image-open='img-a']")!
    .dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true }));
  await handle.settle();
  const box = doc.querySelector(".lightbox");
  assert.ok(box);
  assert.equal(box.getAttribute("role"), "dialog");
  assert.equal(box.getAttribute("aria-modal"), "true");
  assert.equal(box.getAttribute("aria-label"), "Home page after the change");
  assert.equal(box.querySelector("img")?.getAttribute("src"), "blob:a");
  assert.ok(handle.effects().some((e) => e.effect.type === "focusLightbox"));
  box
    .querySelector<HTMLElement>("button[aria-label='Close image']")!
    .dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true }));
  await handle.settle();
  assert.equal(doc.querySelector(".lightbox"), null);
  assert.ok(
    handle
      .effects()
      .some((e) => e.effect.type === "focusImage" && e.effect.id === "img-a"),
    "focus goes back to the thumbnail it came from",
  );
  handle.stop();
  dom.window.close();
});

test("clicking outside the picture closes it, but clicking the picture does not", () => {
  const [open] = update(base, { type: "openImage", id: "img-a" });
  assert.deepEqual(open.lightbox, {
    id: "img-a",
    label: "Home page after the change",
  });
  const [stays] = update(open, { type: "backdropClick", onImage: true });
  assert.ok(stays.lightbox);
  const [closed] = update(open, { type: "backdropClick", onImage: false });
  assert.equal(closed.lightbox, null);
});

test("Escape closes it, and only it: other keys do not", () => {
  assert.equal(closesLightbox({ key: "Escape" }), true);
  assert.equal(closesLightbox({ key: "Enter" }), false);
  assert.equal(
    closesLightbox({ key: "Escape", isComposing: true }),
    false,
    "not while an input method is composing",
  );
});

test("an image whose picture is not loaded cannot be opened, and switching chats closes it", () => {
  const [none] = update(
    { ...base, thumbs: {} },
    { type: "openImage", id: "img-a" },
  );
  assert.equal(none.lightbox, null);
  const [open] = update(base, { type: "openImage", id: "img-a" });
  const [switched] = update(open, { type: "select", id: "s2" });
  assert.equal(switched.lightbox, null);
});

test("an image arriving while the owner watches is fetched for its thumbnail", () => {
  const [, effects] = update({ ...base, thumbs: {} }, {
    type: "event",
    event: shown(9, "A live screenshot", image("img-live")),
  } as never);
  assert.ok(effects.some((e) => e.type === "thumbnail" && e.id === "img-live"));
});

test("navigation follows loaded images, stops at each end, and returns focus to the opener", () => {
  const [open] = update(base, { type: "openImage", id: "img-a" });
  const [first] = update(open, { type: "stepImage", direction: -1 });
  assert.equal(first, open);
  const [next, effects] = update(open, { type: "stepImage", direction: 1 });
  assert.equal(
    next.lightbox?.id,
    "owner-file",
    "skip the picture whose thumbnail has not arrived",
  );
  assert.deepEqual(
    effects,
    [],
    "navigation does not fetch more history or move focus",
  );
  const [end] = update(next, { type: "stepImage", direction: 1 });
  assert.equal(end, next);
  const [, closed] = update(next, { type: "closeImage" });
  assert.deepEqual(closed, [{ type: "focusImage", id: "img-a" }]);
  const [back] = update(next, { type: "stepImage", direction: -1 });
  assert.equal(back.lightbox?.id, "img-a");
  const [arrived] = update(
    { ...open, thumbs: { ...open.thumbs, "img-b": "blob:b" } },
    { type: "stepImage", direction: 1 },
  );
  assert.equal(
    arrived.lightbox?.id,
    "img-b",
    "newly available images take their timeline position",
  );
});

test("navigation buttons show position and remain open when clicked", async () => {
  const { dom, handle } = await mounted(base);
  const doc = dom.window.document;
  doc.querySelector<HTMLElement>("button[data-image-open='img-a']")!.click();
  await handle.settle();
  assert.equal(
    doc.querySelector<HTMLButtonElement>("button[aria-label='Previous image']")
      ?.disabled,
    true,
  );
  assert.equal(doc.querySelector(".lightbox-position")?.textContent, "1 of 2");
  doc
    .querySelector<HTMLButtonElement>("button[aria-label='Next image']")!
    .click();
  await handle.settle();
  assert.equal(
    doc.querySelector(".lightbox img")?.getAttribute("src"),
    "blob:owner",
  );
  assert.equal(doc.querySelector(".lightbox-position")?.textContent, "2 of 2");
  assert.equal(
    doc.querySelector<HTMLButtonElement>("button[aria-label='Next image']")
      ?.disabled,
    true,
  );
  handle.stop();
  dom.window.close();
});

test("arrow keys navigate without consuming keys during text composition", () => {
  assert.deepEqual(lightboxKey({ key: "ArrowLeft" }), {
    type: "stepImage",
    direction: -1,
  });
  assert.deepEqual(lightboxKey({ key: "ArrowRight" }), {
    type: "stepImage",
    direction: 1,
  });
  assert.equal(lightboxKey({ key: "ArrowRight", isComposing: true }), null);
  assert.equal(lightboxKey({ key: "Enter" }), null);
});
