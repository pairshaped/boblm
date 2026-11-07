import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { mountProgram } from "@pairshaped/hypertea/testing";
import { initial, program, type Model } from "../src/app.ts";
import { messageTime } from "../src/message-time.ts";
import type { SessionInfo, StoredEvent } from "../src/generated/contract.ts";

// Built from local parts so the expectations hold in any time zone.
const local = (month: number, day: number, hour: number, minute: number) =>
  new Date(2026, month - 1, day, hour, minute);

test("a message from today shows only the time", () => {
  const now = local(10, 23, 20, 0);
  assert.equal(messageTime(local(10, 23, 19, 45).getTime(), now), "7:45 PM");
  assert.equal(messageTime(local(10, 23, 0, 5).getTime(), now), "12:05 AM");
  assert.equal(messageTime(local(10, 23, 12, 0).getTime(), now), "12:00 PM");
});

test("an earlier day shows the date and time, with no year", () => {
  const now = local(10, 23, 20, 0);
  assert.equal(
    messageTime(local(10, 22, 9, 23).getTime(), now),
    "Oct 22, 9:23 AM",
  );
  assert.equal(
    messageTime(local(1, 3, 16, 10).getTime(), now),
    "Jan 3, 4:10 PM",
  );
  assert.equal(
    messageTime(new Date(2025, 11, 31, 23, 59).getTime(), now),
    "Dec 31, 11:59 PM",
    "last year's message carries no year",
  );
});

test("the label rolls over at local midnight", () => {
  const at = local(10, 23, 23, 59).getTime();
  assert.equal(messageTime(at, local(10, 23, 23, 59)), "11:59 PM");
  assert.equal(messageTime(at, local(10, 24, 0, 1)), "Oct 23, 11:59 PM");
});

test("the time contains no narrow no-break space", () => {
  assert.doesNotMatch(messageTime(Date.now()), / /);
});

const session: SessionInfo = {
  name: "",
  sessionId: "s",
  workingDirectory: "/local/s",
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
    model: "sonnet" as const,
    effort: null,
  },
};
const when = local(10, 22, 9, 23).getTime();
const events: StoredEvent[] = [
  {
    sessionId: "s",
    seq: 1,
    createdAt: when,
    kind: "message_appended",
    payload: {
      uuid: "u1",
      role: "user",
      content: "run `ls`",
      delivery_state: "started",
    },
  },
  {
    sessionId: "s",
    seq: 2,
    createdAt: when + 1000,
    kind: "agent_event",
    payload: {
      type: "assistant",
      message: {
        id: "a1",
        content: [{ type: "text", text: "**Done** with `ls`" }],
      },
    },
  },
  {
    sessionId: "s",
    seq: 3,
    createdAt: when + 2000,
    kind: "agent_event",
    payload: {
      type: "assistant",
      message: {
        id: "a2",
        content: [
          { type: "tool_use", id: "t1", name: "Read", input: { path: "x" } },
        ],
      },
    },
  },
];
const model: Model = {
  ...initial("t"),
  sessions: [session],
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

test("owner and agent messages get a copy button and a time; tool-only rows get neither", async () => {
  const { dom, handle } = await mounted(model);
  const doc = dom.window.document;
  const user = doc.querySelector(".message.user .message-actions");
  const agent = doc.querySelector(".message.assistant .message-actions");
  assert.ok(user && agent, "both kinds of message have an action row");
  for (const row of [user, agent]) {
    assert.ok(row.querySelector("button[aria-label='Copy message']"));
    assert.match(
      row.querySelector("time")?.textContent ?? "",
      /^Oct 22, 9:2\d AM$/,
    );
    assert.ok(row.querySelector("time")?.getAttribute("datetime"));
  }
  const rows = [...doc.querySelectorAll(".message")];
  const toolOnly = rows.find((r) => r.textContent?.includes("Read call"));
  assert.ok(toolOnly, "the tool call row exists");
  assert.equal(toolOnly.querySelector(".message-actions"), null);
  handle.stop();
  dom.window.close();
});

test("copying an agent message copies its raw markdown, not the rendered text", async () => {
  const { dom, handle } = await mounted(model);
  const button = dom.window.document.querySelector<HTMLElement>(
    ".message.assistant button[aria-label='Copy message']",
  );
  assert.ok(button);
  button.dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true }));
  await handle.settle();
  const copy = handle
    .effects()
    .map((e) => e.effect)
    .find((e) => e.type === "copy");
  assert.ok(copy && copy.type === "copy");
  assert.equal(copy.text, "**Done** with `ls`");
  handle.stop();
  dom.window.close();
});

for (const role of ["user", "assistant"]) {
  test(`${role} messages collapse only through their control and reopen without losing content`, async () => {
    const { dom, handle } = await mounted(model);
    const doc = dom.window.document;
    const row = doc.querySelector<HTMLElement>(`.message.${role}`)!;
    const content = row.querySelector<HTMLElement>(
      role === "user" ? "pre" : ".markdown",
    )!;
    const original = content.textContent;
    content.click();
    await handle.settle();
    assert.equal(row.classList.contains("message-collapsed"), false);
    row
      .querySelector<HTMLElement>("button[aria-label='Collapse message']")!
      .click();
    await handle.settle();
    assert.equal(row.classList.contains("message-collapsed"), true);
    assert.ok(row.querySelector(".message-preview")?.textContent);
    const expand = row.querySelector<HTMLElement>(
      "button[aria-label='Expand message']",
    )!;
    assert.equal(expand.getAttribute("aria-expanded"), "false");
    assert.equal(doc.querySelectorAll(".message-collapsed").length, 1);
    content.click();
    await handle.settle();
    assert.equal(row.classList.contains("message-collapsed"), true);
    row
      .querySelector<HTMLElement>("button[aria-label='Copy message']")!
      .click();
    await handle.settle();
    assert.ok(
      handle
        .effects()
        .some(
          ({ effect }) =>
            effect.type === "copy" &&
            effect.text ===
              (role === "user" ? "run `ls`" : "**Done** with `ls`"),
        ),
    );
    expand.click();
    await handle.settle();
    assert.equal(row.classList.contains("message-collapsed"), false);
    assert.equal(row.querySelector(".message-preview"), null);
    assert.equal(content.textContent, original);
    handle.stop();
    dom.window.close();
  });
}
