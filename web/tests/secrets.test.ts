import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { mountProgram } from "@pairshaped/hypertea/testing";
import {
  initial,
  pendingSecrets,
  program,
  update,
  type Model,
} from "../src/app.ts";
import type { SessionInfo, StoredEvent } from "../src/generated/contract.ts";

const R1 = "30000000-0000-4000-8000-000000000001";
const R2 = "30000000-0000-4000-8000-000000000002";
const VALUE = "correct-horse-battery-STAPLE-9141";
const requested = (seq: number, id: string, key: string[]): StoredEvent => ({
  sessionId: "s",
  seq,
  createdAt: 1,
  kind: "secret_requested",
  payload: {
    request_id: id,
    file: "/home/me/secrets.yaml",
    key,
    expires_at: 1,
  },
});
const resolved = (seq: number, id: string, status: string): StoredEvent => ({
  sessionId: "s",
  seq,
  createdAt: 1,
  kind: "secret_resolved",
  payload: { request_id: id, status },
});
const chat: SessionInfo = {
  name: "",
  sessionId: "s",
  projectId: "p1",
  agent: "claude",
  workingDirectory: "/work",
  agentSessionId: null,
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
  ...initial("t"),
  sessions: [chat],
  active: "s",
  models: [
    { model: "sonnet", label: "sonnet", agent: "claude", efforts: ["low"] },
  ],
  projects: [{ projectId: "p1", name: "work", path: "/work", createdAt: 1 }],
  events: {
    s: [
      requested(1, R1, ["db", "password"]),
      requested(2, R2, ["api", "token"]),
      resolved(3, R1, "saved"),
    ],
  },
};

test("a request is waiting until a resolution event for it arrives", () => {
  assert.deepEqual(
    pendingSecrets(base.events.s ?? []).map((r) => r.requestId),
    [R2],
  );
  assert.deepEqual(
    pendingSecrets([...(base.events.s ?? []), resolved(4, R2, "expired")]),
    [],
  );
  assert.equal(pendingSecrets([])[0], undefined);
});

test("saving sends the value once and keeps no copy of it in the page's state", () => {
  const [next, effects] = update(base, {
    type: "submitSecret",
    requestId: R2,
    value: VALUE,
  });
  assert.deepEqual(effects, [
    {
      type: "request",
      name: "submitSecret",
      sessionId: R2,
      body: { requestId: R2, value: VALUE },
    },
  ]);
  assert.ok(!JSON.stringify(next).includes(VALUE));
  assert.deepEqual(
    update(base, { type: "submitSecret", requestId: R2, value: "" })[1],
    [],
    "nothing to save",
  );
});

test("declining sends a refusal, not a value", () => {
  const [, effects] = update(base, { type: "declineSecret", requestId: R2 });
  assert.deepEqual(effects, [
    {
      type: "request",
      name: "submitSecret",
      sessionId: R2,
      body: { requestId: R2, cancel: true },
    },
  ]);
});

test("a save that failed or arrived too late says why; one that worked is shown by the chat itself", () => {
  const reply = (status: string, message: string) =>
    update(base, {
      type: "reply",
      name: "submitSecret",
      sessionId: R2,
      outcome: { kind: "replied", reply: { status, message } },
    })[0];
  assert.equal(reply("saved", "Saved.").error, "");
  assert.equal(reply("canceled", "Declined.").error, "");
  assert.equal(
    reply("failed", "SOPS refused (exit status: 1). Nothing was changed.")
      .error,
    "SOPS refused (exit status: 1). Nothing was changed.",
  );
  assert.match(reply("expired", "This request expired.").error, /expired/);
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

test("a waiting request shows where the value goes and a masked input, and goes away when answered", async () => {
  const { dom, handle } = await mounted(base);
  const document = dom.window.document;
  const cards = [...document.querySelectorAll("[data-secret-request]")];
  assert.equal(cards.length, 1);
  assert.equal(cards[0]!.getAttribute("data-secret-request"), R2);
  assert.match(cards[0]!.textContent ?? "", /api\.token/);
  assert.match(cards[0]!.textContent ?? "", /\/home\/me\/secrets\.yaml/);
  const input = cards[0]!.querySelector<HTMLInputElement>("input");
  assert.equal(input?.type, "password");
  assert.equal(input?.getAttribute("autocomplete"), "off");
  assert.ok(cards[0]!.querySelector("button[type=submit]"));
  assert.ok(cards[0]!.querySelector("[data-decline-secret]"));
  handle.stop();
  dom.window.close();
  const done = await mounted({
    ...base,
    events: { s: [...(base.events.s ?? []), resolved(4, R2, "saved")] },
  });
  assert.equal(
    done.dom.window.document.querySelectorAll("[data-secret-request]").length,
    0,
  );
  done.handle.stop();
  done.dom.window.close();
});
