import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { mountProgram } from "@pairshaped/hypertea/testing";
import { initial, program, type Model } from "../src/app.ts";
import type {
  DeliveryState,
  SessionInfo,
  StoredEvent,
} from "../src/generated/contract.ts";

const status = {
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
};
const chat = (extra: Partial<SessionInfo["status"]>): SessionInfo => ({
  name: "Chat",
  sessionId: "s",
  workingDirectory: "/local/s",
  agentSessionId: null,
  agent: "claude" as const,
  projectId: "p1",
  createdAt: 1,
  status: { ...status, ...extra },
  subagents: { open: 2, questions: 0, failures: 0, reportFailures: 0 },
});
const message = (
  seq: number,
  uuid: string,
  state: DeliveryState,
  expiredFrom?: "queued" | "uncertain",
): StoredEvent => ({
  sessionId: "s",
  seq,
  createdAt: 1,
  kind: "message_appended",
  payload: {
    uuid,
    role: "user",
    content: `text of ${uuid}`,
    delivery_state: state,
    ...(expiredFrom ? { expired_from: expiredFrom } : {}),
  },
});
const model = (extra: Partial<SessionInfo["status"]>, events: StoredEvent[]) =>
  ({
    ...initial("t"),
    sessions: [chat(extra)],
    projects: [{ projectId: "p1", name: "P", path: "/p", createdAt: 1 }],
    models: [
      {
        model: "sonnet",
        label: "sonnet",
        agent: "claude" as const,
        efforts: ["low"],
      },
    ],
    active: "s",
    events: { s: events },
  }) as Model;

async function mounted(flags: Model) {
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
    flags,
    node: dom.window.document.querySelector<HTMLElement>("#app")!,
    program,
  });
  await handle.settle();
  return { dom, handle };
}
const sidebar = (dom: JSDOM) =>
  dom.window.document.querySelector(".chat-status")!;

test("the sidebar says held, not queued, when the queue is held, and names subagents", async () => {
  const held = await mounted(model({ queued: 2, queueHeld: true }, []));
  assert.equal(sidebar(held.dom).textContent, "2 held · 2 subagents");
  assert.match(sidebar(held.dom).getAttribute("title")!, /2 held messages/);
  held.handle.stop();
  held.dom.window.close();
  const flowing = await mounted(model({ queued: 1, queueHeld: false }, []));
  assert.equal(sidebar(flowing.dom).textContent, "1 queued · 2 subagents");
  assert.match(
    sidebar(flowing.dom).getAttribute("title")!,
    /1 queued message;/,
  );
  flowing.handle.stop();
  flowing.dom.window.close();
});

test("a held queue shows the last driver error beside Resume", async () => {
  const reason =
    "Claude could not be started, so the queued messages are held (cause: the binary is missing).";
  const { dom, handle } = await mounted(
    model({ queued: 1, queueHeld: true, heldReason: reason }, []),
  );
  const note = dom.window.document.querySelector("[data-testid=held-reason]");
  assert.equal(note?.textContent, reason);
  const buttons = [...dom.window.document.querySelectorAll("button")].map(
    (b) => b.textContent,
  );
  assert.ok(buttons.includes("Resume"));
  handle.stop();
  dom.window.close();
});

test("a held queue without a recorded error shows no reason, and a flowing queue shows no Resume", async () => {
  const plain = await mounted(model({ queued: 1, queueHeld: true }, []));
  assert.equal(
    plain.dom.window.document.querySelector("[data-testid=held-reason]"),
    null,
  );
  plain.handle.stop();
  plain.dom.window.close();
  const flowing = await mounted(
    model({ queued: 1, queueHeld: false, heldReason: "stale" }, []),
  );
  assert.equal(
    flowing.dom.window.document.querySelector("[data-testid=held-reason]"),
    null,
  );
  flowing.handle.stop();
  flowing.dom.window.close();
});

test("an expired message stays in place with a plain note, and an unconfirmed one says it may have arrived", async () => {
  const { dom, handle } = await mounted(
    model({}, [
      message(1, "queued-one", "expired", "queued"),
      message(2, "lost-one", "expired", "uncertain"),
      message(3, "fresh-one", "queued"),
    ]),
  );
  const notes = (uuid: string) =>
    dom.window.document.querySelector(`[data-expired-uuid="${uuid}"]`)
      ?.textContent;
  assert.equal(notes("queued-one"), "Expired unsent after 6 hours.");
  assert.match(notes("lost-one")!, /Expired unsent after 6 hours\./);
  assert.match(notes("lost-one")!, /may already have been received/);
  assert.equal(notes("fresh-one"), undefined);
  const rows = [...dom.window.document.querySelectorAll("article.message")];
  assert.equal(rows.length, 3, "expired messages are marked, never removed");
  assert.equal(
    dom.window.document.querySelector(".uncertain-banner"),
    null,
    "an expired message is no longer waiting on the owner",
  );
  handle.stop();
  dom.window.close();
});

test("a delivery change to expired marks the row the same way", async () => {
  const { dom, handle } = await mounted(
    model({}, [
      message(1, "late", "queued"),
      {
        sessionId: "s",
        seq: 2,
        createdAt: 2,
        kind: "delivery_changed",
        payload: {
          uuid: "late",
          delivery_state: "expired",
          expired_from: "uncertain",
          retry_count: null,
        },
      },
    ]),
  );
  assert.match(
    dom.window.document.querySelector("[data-expired-uuid=late]")!.textContent!,
    /may already have been received/,
  );
  handle.stop();
  dom.window.close();
});
