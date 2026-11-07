import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { mountProgram } from "@pairshaped/hypertea/testing";
import { initial, program, update, type Model } from "../src/app.ts";
import type { SessionInfo } from "../src/generated/contract.ts";

const status = {
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
  model: "sonnet" as const,
  effort: null,
};
const session = (id: string): SessionInfo => ({
  name: "",
  sessionId: id,
  workingDirectory: `/local/${id}`,
  agentSessionId: null,
  agent: "claude" as const,
  projectId: "p1",
  createdAt: 1,
  status,
});
const base: Model = {
  ...initial("t"),
  sessions: [session("s"), session("other")],
  models: [
    {
      model: "sonnet",
      label: "sonnet",
      agent: "claude" as const,
      efforts: ["low"],
    },
  ],
  active: "s",
  drafts: { s: "hello" },
};
const failed = (outcome: object) => ({
  type: "reply" as const,
  name: "sendMessage",
  sessionId: "s",
  outcome: outcome as never,
});

test("a send the server refused keeps the draft, shows the error by the composer and offers a fresh retry", () => {
  const [sending] = update(base, { type: "send", id: "first" });
  const [refused, effects] = update(
    sending,
    failed({
      kind: "rejected",
      error: { code: "invalid_request", message: "invalid message UUID" },
    }),
  );
  assert.equal(refused.drafts.s, "hello");
  assert.deepEqual(refused.pending, {});
  assert.equal(refused.sendError.s, "invalid message UUID");
  assert.equal(
    refused.error,
    "",
    "the global error line is not used for sends",
  );
  assert.deepEqual(
    effects,
    [{ type: "persistUncertain", uncertain: {} }],
    "a refusal clears the saved copy of the in-flight send",
  );
  // The server said it did not accept, so Retry is an ordinary send with a new UUID.
  const [retrying, request] = update(refused, { type: "send", id: "second" });
  assert.equal(retrying.sendError.s, undefined);
  assert.equal(
    (
      request.find((e) => e.type === "request") as {
        body: { messageId: string };
      }
    ).body.messageId,
    "second",
  );
});

test("an internal server error may follow a saved message, so it is held like a lost reply", () => {
  const [sending] = update(base, { type: "send", id: "first" });
  const [held, effects] = update(
    sending,
    failed({
      kind: "rejected",
      error: { code: "internal", message: "Server operation failed" },
    }),
  );
  assert.deepEqual(held.uncertain.s?.id, "first");
  assert.equal(effects[0]?.type, "persistUncertain");
  const [, request] = update(held, { type: "retrySend" });
  assert.equal(
    (request[0] as { body: { messageId: string } }).body.messageId,
    "first",
  );
});

test("an uncertain send is persisted, retried under the same UUID and forgotten on success", () => {
  const [sending] = update(base, { type: "send", id: "first" });
  const [held, effects] = update(sending, failed({ kind: "uncertain" }));
  assert.match(
    held.sendError.s ?? "",
    /cannot tell whether it received your message/i,
  );
  assert.deepEqual(effects, [
    {
      type: "persistUncertain",
      uncertain: { s: { id: "first", content: "hello", attachments: [] } },
    },
  ]);
  const [retrying, request] = update(held, { type: "retrySend" });
  assert.equal(retrying.sendError.s, undefined);
  assert.equal(
    (request[0] as { body: { messageId: string } }).body.messageId,
    "first",
  );
  const [done, after] = update(retrying, {
    type: "reply",
    name: "sendMessage",
    sessionId: "s",
    outcome: {
      kind: "replied",
      reply: { messageId: "first", accepted: true, status },
    },
  });
  assert.deepEqual(done.uncertain, {});
  assert.ok(
    after.some(
      (e) =>
        e.type === "persistUncertain" && Object.keys(e.uncertain).length === 0,
    ),
  );
});

test("the failure belongs to its session", () => {
  const [sending] = update(base, { type: "send", id: "first" });
  const [held] = update(sending, failed({ kind: "uncertain" }));
  const [other] = update(held, { type: "select", id: "other" });
  assert.equal(other.error, "");
  assert.equal(other.sendError.other, undefined);
  assert.ok(other.sendError.s, "still waiting on the first session");
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

test("a send held before a reload still offers Retry with its original UUID", async () => {
  const held = { id: "kept-uuid", content: "hello", attachments: [] };
  const model = initial("t", { s: "hello" }, {}, { s: held });
  const { dom, handle } = await mounted({
    ...model,
    sessions: [session("s")],
    models: [
      {
        model: "sonnet",
        label: "sonnet",
        agent: "claude" as const,
        efforts: ["low"],
      },
    ],
    active: "s",
  });
  const banner = dom.window.document.querySelector('.compose [role="alert"]');
  assert.match(banner?.textContent ?? "", /Earlier send/);
  const retry = [...(banner?.querySelectorAll("button") ?? [])].find(
    (b) => b.textContent === "Retry",
  );
  assert.ok(retry, "Retry sits beside the error");
  retry.dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true }));
  await handle.settle();
  const request = handle
    .effects()
    .map((e) => e.effect)
    .find((e) => e.type === "request" && e.name === "sendMessage");
  assert.ok(request && request.type === "request");
  assert.equal((request.body as { messageId: string }).messageId, "kept-uuid");
  assert.equal(
    dom.window.document.querySelectorAll(".compose .send-row button").length >=
      1,
    true,
  );
  handle.stop();
  dom.window.close();
});

test("while a send is unconfirmed nothing else is sent; it can be retried or discarded", () => {
  const [sending] = update(base, { type: "send", id: "first" });
  const [held] = update(sending, failed({ kind: "uncertain" }));
  const edited = { ...held, drafts: { s: "a different message" } };
  const [blocked, effects] = update(edited, { type: "send", id: "second" });
  assert.deepEqual(effects, [], "a different send must not start");
  assert.deepEqual(blocked.uncertain.s?.id, "first", "the held send is kept");
  assert.match(blocked.sendError.s ?? "", /unconfirmed/i);
  assert.match(blocked.sendError.s ?? "", /retry|discard/i);
  const [discarded, persisted] = update(blocked, { type: "discardHeld" });
  assert.deepEqual(discarded.uncertain, {});
  assert.equal(discarded.sendError.s, undefined);
  assert.equal(discarded.drafts.s, "a different message", "the draft stays");
  assert.deepEqual(persisted, [{ type: "persistUncertain", uncertain: {} }]);
  const [free, request] = update(discarded, { type: "send", id: "second" });
  assert.equal(
    (
      request.find((e) => e.type === "request") as {
        body: { messageId: string };
      }
    ).body.messageId,
    "second",
  );
  assert.deepEqual(free.pending.s?.id, "second");
});

test("a send is saved before it is dispatched, so a reload mid-flight cannot lose its ID", () => {
  const [sending, effects] = update(base, { type: "send", id: "first" });
  assert.deepEqual(
    effects.map((e) => e.type),
    ["persistUncertain", "request"],
    "the ID is stored before the request goes out",
  );
  assert.deepEqual(
    (effects[0] as { uncertain: Record<string, { id: string }> }).uncertain.s
      ?.id,
    "first",
  );
  // A refusal means it was never accepted: the saved copy goes away.
  const [, refused] = update(
    sending,
    failed({
      kind: "rejected",
      error: { code: "invalid_request", message: "no" },
    }),
  );
  assert.deepEqual(refused, [{ type: "persistUncertain", uncertain: {} }]);
});

test("locking the app keeps an unconfirmed send and an in-flight one", () => {
  const [sending] = update(base, { type: "send", id: "first" });
  const [locked] = update(sending, { type: "logout" });
  assert.equal(locked.token, "");
  assert.equal(
    locked.uncertain.s?.id,
    "first",
    "in flight at the moment of locking",
  );
  assert.match(locked.sendError.s ?? "", /Earlier send/);
});

test("an unavailable reply can follow a saved message, so it is held like an internal error", () => {
  const [sending] = update(base, { type: "send", id: "first" });
  const [held] = update(
    sending,
    failed({
      kind: "rejected",
      error: { code: "unavailable", message: "later" },
    }),
  );
  assert.equal(held.uncertain.s?.id, "first");
});
