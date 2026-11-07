import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { mountProgram } from "@pairshaped/hypertea/testing";
import {
  initial,
  program,
  chatRows,
  update,
  type Model,
  type Msg,
} from "../src/app.ts";
import type { SessionInfo, StoredEvent } from "../src/generated/contract.ts";
import { putUpload } from "../src/http.ts";

const session: SessionInfo = {
  name: "",
  sessionId: "s",
  workingDirectory: "/local/project",
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
    model: "sonnet",
    effort: null,
  },
};
const base: Model = {
  ...initial("token"),
  sessions: [session],
  models: [
    {
      model: "sonnet",
      label: "sonnet",
      agent: "claude" as const,
      efforts: ["low", "high"],
    },
  ],
  active: "s",
};
const U1 = "10000000-0000-4000-8000-000000000001";
const U2 = "10000000-0000-4000-8000-000000000002";
const png = new File([new Uint8Array(2048)], "shot.png", { type: "image/png" });
const text = new File(["hello"], "notes.txt", { type: "text/plain" });

function step(model: Model, msg: Msg) {
  return update(model, msg);
}

test("choosing files adds uploading chips, starts uploads and previews images", () => {
  const [model, effects] = step(base, {
    type: "attach",
    items: [
      { id: U1, file: png },
      { id: U2, file: text },
    ],
  });
  assert.deepEqual(
    model.attachments.s?.map((a) => [a.id, a.name, a.contentType, a.state]),
    [
      [U1, "shot.png", "image/png", "uploading"],
      [U2, "notes.txt", "text/plain", "uploading"],
    ],
  );
  assert.deepEqual(
    effects.map((e) => e.type),
    ["upload", "thumbnail", "upload"],
  );
});

test("limits: ten per message, 50 MiB each; archived chats can prepare uploads", () => {
  const files = Array.from({ length: 12 }, (_, i) => ({
    id: `10000000-0000-4000-8000-0000000001${String(i).padStart(2, "0")}`,
    file: text,
  }));
  const [many, effects] = step(base, { type: "attach", items: files });
  assert.equal(many.attachments.s?.length, 10);
  assert.equal(effects.length, 10);
  assert.match(many.error, /At most 10/);
  const huge = new File([new Uint8Array(1)], "huge.bin");
  Object.defineProperty(huge, "size", { value: 52_428_801 });
  const [refused, none] = step(base, {
    type: "attach",
    items: [{ id: U1, file: huge }],
  });
  assert.deepEqual(refused.attachments.s ?? [], []);
  assert.deepEqual(none, []);
  assert.match(refused.error, /huge.bin is larger than 50 MiB/);
  const archived: Model = {
    ...base,
    sessions: [{ ...session, status: { ...session.status, archived: true } }],
  };
  assert.equal(
    step(archived, { type: "attach", items: [{ id: U1, file: text }] })[1]
      .length,
    1,
  );
});

test("send waits for uploads, then carries ready attachments and clears them on success", () => {
  let [model] = step(base, {
    type: "attach",
    items: [{ id: U1, file: text }],
  });
  model = { ...model, drafts: { s: "see this" } };
  const [blocked, none] = step(model, { type: "send", id: "m1" });
  assert.deepEqual(none, []);
  assert.match(blocked.error, /Wait for uploads/);
  [model] = step(model, {
    type: "uploaded",
    sessionId: "s",
    id: U1,
    outcome: {
      kind: "replied",
      reply: { id: U1, name: "notes.txt", contentType: "text/plain", size: 5 },
    },
  });
  assert.equal(model.attachments.s?.[0]?.state, "ready");
  const [sending, effects] = step(model, { type: "send", id: "m1" });
  assert.deepEqual(effects, [
    {
      type: "persistUncertain",
      uncertain: {
        s: { id: "m1", content: "see this", attachments: [U1] },
      },
    },
    {
      type: "request",
      name: "sendMessage",
      sessionId: "s",
      body: {
        sessionId: "s",
        messageId: "m1",
        content: "see this",
        attachments: [U1],
      },
    },
  ]);
  // Attachments alone are enough.
  const [only] = step(
    { ...model, drafts: { s: "" } },
    { type: "send", id: "m2" },
  );
  assert.deepEqual(only.pending.s?.attachments, [U1]);
  // A failed reply keeps them for another try; success clears them.
  const [failed] = step(sending, {
    type: "reply",
    name: "sendMessage",
    sessionId: "s",
    outcome: { kind: "uncertain" },
  });
  assert.equal(failed.attachments.s?.length, 1);
  assert.deepEqual(failed.uncertain.s?.attachments, [U1]);
  const [, retry] = step(failed, { type: "retrySend" });
  assert.deepEqual(
    (retry[0] as { body: { attachments: string[] } }).body.attachments,
    [U1],
  );
  const [done, after] = step(sending, {
    type: "reply",
    name: "sendMessage",
    sessionId: "s",
    outcome: {
      kind: "replied",
      reply: { messageId: "m1", accepted: true, status: session.status },
    },
  });
  assert.deepEqual(done.attachments.s, []);
  assert.ok(after.some((e) => e.type === "discard" && e.id === U1));
});

test("failed uploads can be retried with the same id or removed", () => {
  let [model] = step(base, { type: "attach", items: [{ id: U1, file: png }] });
  [model] = step(model, { type: "thumb", id: U1, url: "blob:local" });
  [model] = step(model, {
    type: "uploaded",
    sessionId: "s",
    id: U1,
    outcome: { kind: "uncertain" },
  });
  assert.equal(model.attachments.s?.[0]?.state, "failed");
  const [retrying, effects] = step(model, { type: "retryUpload", id: U1 });
  assert.equal(retrying.attachments.s?.[0]?.state, "uploading");
  assert.equal(retrying.attachments.s?.[0]?.error, undefined);
  assert.deepEqual(effects, [{ type: "upload", sessionId: "s", id: U1 }]);
  const [removed, discard] = step(model, { type: "removeAttachment", id: U1 });
  assert.deepEqual(removed.attachments.s, []);
  assert.deepEqual(removed.thumbs, {});
  assert.deepEqual(discard, [
    { type: "discard", id: U1, url: "blob:local" },
    { type: "persistAttachments", attachments: {} },
  ]);
  // A reply for a removed chip changes nothing.
  assert.equal(
    step(removed, {
      type: "uploaded",
      sessionId: "s",
      id: U1,
      outcome: { kind: "uncertain" },
    })[0],
    removed,
  );
});

const appended: StoredEvent = {
  sessionId: "s",
  seq: 1,
  createdAt: 1,
  kind: "message_appended",
  payload: {
    uuid: "m",
    role: "user",
    content: "",
    delivery_state: "queued",
    attachments: [
      { id: U1, name: "shot.png", contentType: "image/png", size: 2048 },
      { id: U2, name: "notes.txt", contentType: "text/plain", size: 5 },
    ],
  },
};

test("sent attachments appear in history and image thumbnails are fetched once", () => {
  assert.equal(chatRows([appended])[0]?.attachments?.length, 2);
  const [model, effects] = step(base, { type: "event", event: appended });
  assert.deepEqual(effects, [{ type: "thumbnail", sessionId: "s", id: U1 }]);
  const [withThumb] = step(model, { type: "thumb", id: U1, url: "blob:x" });
  assert.deepEqual(
    step(withThumb, { type: "event", event: { ...appended, seq: 2 } })[1],
    [],
  );
  assert.deepEqual(
    step(base, { type: "download", id: U2, name: "notes.txt" })[1],
    [{ type: "download", sessionId: "s", id: U2, name: "notes.txt" }],
  );
});

async function mounted(model: Model) {
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
    flags: model,
    node: dom.window.document.querySelector<HTMLElement>("#app")!,
    program,
  });
  await handle.settle();
  return { dom, handle };
}

test("the composer shows chips, keeps the textarea focused and pasting an image attaches it", async () => {
  const [model] = step(base, {
    type: "attach",
    items: [
      { id: U1, file: png },
      { id: U2, file: text },
    ],
  });
  const withState: Model = {
    ...model,
    thumbs: { [U1]: "blob:thumb" },
    attachments: {
      s: model.attachments.s!.map((a, i) =>
        i === 1 ? { ...a, state: "failed", error: "Too big" } : a,
      ),
    },
  };
  const { dom, handle } = await mounted(withState);
  const document = dom.window.document;
  const chips = document.querySelectorAll(".compose .chip");
  assert.equal(chips.length, 2);
  assert.equal(
    chips[0]?.querySelector("img")?.getAttribute("src"),
    "blob:thumb",
  );
  assert.match(chips[0]?.textContent ?? "", /shot\.png.*Uploading/);
  assert.match(chips[1]?.textContent ?? "", /notes\.txt.*Too big/);
  assert.ok(chips[1]?.querySelector("button")?.textContent?.includes("Retry"));
  assert.ok(document.querySelector("#attach-input"));
  assert.equal(
    document.querySelector<HTMLButtonElement>(".attach")?.disabled,
    false,
  );

  const textarea = document.querySelector("textarea")!;
  textarea.focus();
  const paste = new dom.window.Event("paste", {
    bubbles: true,
    cancelable: true,
  });
  Object.defineProperty(paste, "clipboardData", {
    value: {
      files: [new dom.window.File(["x"], "pasted.png", { type: "image/png" })],
    },
  });
  textarea.dispatchEvent(paste);
  await handle.settle();
  assert.equal(paste.defaultPrevented, true);
  assert.equal(document.activeElement, textarea);
  assert.equal(document.querySelectorAll(".compose .chip").length, 3);
  assert.ok(handle.effects().some((e) => e.effect.type === "upload"));

  const plain = new dom.window.Event("paste", {
    bubbles: true,
    cancelable: true,
  });
  Object.defineProperty(plain, "clipboardData", { value: { files: [] } });
  textarea.dispatchEvent(plain);
  assert.equal(plain.defaultPrevented, false, "text paste is left alone");
  handle.stop();
  dom.window.close();
});

test("uploads send the raw file with the name and the bearer token", async () => {
  const seen: { url: string; init: RequestInit }[] = [];
  const fake = (async (url: string, init: RequestInit) => {
    seen.push({ url, init });
    return new Response(
      JSON.stringify({
        id: U1,
        name: "a b.txt",
        contentType: "text/plain",
        size: 5,
      }),
      { status: 200 },
    );
  }) as unknown as typeof fetch;
  const file = new File(["hello"], "a b é.txt", { type: "text/plain" });
  const outcome = await putUpload("s/1", U1, file, "tok", (v) => v, fake);
  assert.equal(outcome.kind, "replied");
  assert.equal(seen[0]?.url, `/sessions/s%2F1/uploads/${U1}`);
  const headers = seen[0]?.init.headers as Record<string, string>;
  assert.equal(seen[0]?.init.method, "PUT");
  assert.equal(seen[0]?.init.body, file);
  assert.equal(headers["X-Filename"], "a%20b%20%C3%A9.txt");
  assert.equal(headers["Content-Type"], "text/plain");
  assert.equal(headers.Authorization, "Bearer tok");
  const refused = (async () =>
    new Response(JSON.stringify({ code: "upload_conflict", message: "no" }), {
      status: 409,
    })) as unknown as typeof fetch;
  assert.equal(
    (await putUpload("s", U1, file, "t", (v) => v, refused)).kind,
    "rejected",
  );
  const dropped = (async () => {
    throw new Error("network");
  }) as unknown as typeof fetch;
  assert.equal(
    (await putUpload("s", U1, file, "t", (v) => v, dropped)).kind,
    "uncertain",
  );
});

test("removing a file named by a held send drops that send, and ready files survive a reload", () => {
  let [model] = step(base, { type: "attach", items: [{ id: U1, file: text }] });
  const ready = {
    kind: "replied" as const,
    reply: { id: U1, name: "notes.txt", contentType: "text/plain", size: 5 },
  };
  let effects;
  [model, effects] = step(model, {
    type: "uploaded",
    sessionId: "s",
    id: U1,
    outcome: ready,
  });
  // Only finished uploads are persisted for the next load.
  assert.deepEqual(effects, [
    {
      type: "persistAttachments",
      attachments: { s: [model.attachments.s![0]] },
    },
  ]);
  const restored = initial("t", {}, { s: model.attachments.s! });
  assert.equal(restored.attachments.s?.[0]?.state, "ready");
  model = { ...model, drafts: { s: "see this" } };
  let sending;
  [sending] = step(model, { type: "send", id: "m1" });
  [model] = step(sending, {
    type: "reply",
    name: "sendMessage",
    sessionId: "s",
    outcome: { kind: "uncertain" },
  });
  assert.ok(model.uncertain.s);
  [model] = step(model, { type: "removeAttachment", id: U1 });
  assert.equal(model.uncertain.s, undefined, "the held send is dropped");
  assert.match(model.sendError.s ?? "", /may still have arrived/);
  assert.deepEqual(step(model, { type: "retrySend" })[1], []);
});

test("locking keeps the draft, finished uploads and their previews", () => {
  const ready = {
    id: U1,
    name: "notes.txt",
    contentType: "text/plain",
    size: 5,
    state: "ready" as const,
  };
  const model: Model = {
    ...base,
    drafts: { s: "text" },
    thumbs: { [U1]: "blob:x" },
    attachments: { s: [ready, { ...ready, id: U2, state: "uploading" }] },
  };
  const [locked] = step(model, { type: "logout" });
  assert.equal(locked.token, "");
  assert.deepEqual(locked.drafts, { s: "text" });
  assert.deepEqual(locked.attachments, { s: [ready] });
  assert.deepEqual(locked.thumbs, { [U1]: "blob:x" });
});
