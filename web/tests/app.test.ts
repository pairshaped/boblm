import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { mountProgram } from "@pairshaped/hypertea/testing";
import { initial, program, chatRows, update, type Model } from "../src/app.ts";
import type { SessionInfo, StoredEvent } from "../src/generated/contract.ts";
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
    turnActive: true,
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
async function fixture() {
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
  const model: Model = {
    ...initial("fixture-token"),
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
  const handle = mountProgram({
    flags: model,
    node: dom.window.document.querySelector<HTMLElement>("#app")!,
    program,
  });
  await handle.settle();
  handle.takeEffect(); // The connected fixture starts with a chat-list read.
  return { dom, handle, model };
}
test("streaming preserves exact textarea text, selection and focus; sending uses readOnly", async () => {
  const { dom, handle } = await fixture();
  const textarea = dom.window.document.querySelector("textarea")!;
  textarea.focus();
  textarea.value = "line one\nline two ☕";
  textarea.setSelectionRange(3, 11, "backward");
  textarea.dispatchEvent(new dom.window.Event("input", { bubbles: true }));
  await handle.settle();
  for (let seq = 1; seq <= 20; seq++)
    handle.dispatch({
      type: "event",
      event: {
        kind: "agent_event",
        sessionId: "s",
        seq,
        createdAt: 1,
        payload: {
          type: "assistant",
          message: {
            id: `a${seq}`,
            content: [{ type: "text", text: `Stream ${seq}` }],
          },
        },
      },
    });
  await handle.settle();
  assert.equal(textarea.value, "line one\nline two ☕");
  assert.equal(textarea.selectionStart, 3);
  assert.equal(textarea.selectionEnd, 11);
  assert.equal(textarea.selectionDirection, "backward");
  assert.equal(dom.window.document.activeElement, textarea);
  textarea.dispatchEvent(
    new dom.window.KeyboardEvent("keydown", {
      key: "Enter",
      bubbles: true,
      cancelable: true,
    }),
  );
  await handle.settle();
  assert.equal(textarea.readOnly, true);
  assert.equal(textarea.disabled, false);
  assert.equal(dom.window.document.activeElement, textarea);
  assert.equal(handle.takeEffect()?.effect.type, "persist");
  assert.equal(
    handle.takeEffect()?.effect.type,
    "persistUncertain",
    "the send is saved before it goes out",
  );
  const send = handle.takeEffect()?.effect;
  assert.ok(send?.type === "request");
  assert.equal(send.name, "sendMessage");
  handle.stop();
  dom.window.close();
});
test("Shift+Enter and composition do not send, pending selection is visible", async () => {
  const { dom, handle } = await fixture();
  const textarea = dom.window.document.querySelector("textarea")!;
  for (const options of [{ shiftKey: true }, { isComposing: true }]) {
    const event = new dom.window.KeyboardEvent("keydown", {
      key: "Enter",
      bubbles: true,
      cancelable: true,
      ...options,
    });
    textarea.dispatchEvent(event);
    assert.equal(event.defaultPrevented, false);
  }
  await handle.settle();
  assert.equal(handle.effects().length, 0);
  handle.dispatch({
    type: "event",
    event: {
      kind: "state_summary",
      sessionId: "s",
      seq: 1,
      createdAt: 1,
      payload: { ...session.status, selectionPending: true, model: "opus" },
    },
  });
  await handle.settle();
  assert.equal(
    dom.window.document.querySelector('[data-testid="pending-selection"]')
      ?.textContent,
    "Applies next turn",
  );
  handle.stop();
  dom.window.close();
});
test("uncertain send preserves original UUID on explicit retry and retains the draft", () => {
  const model = { ...initial(), active: "s", drafts: { s: "hello" } };
  const [sent] = update(model, { type: "send", id: "client-uuid" });
  const [failed] = update(sent, {
    type: "reply",
    name: "sendMessage",
    sessionId: "s",
    outcome: { kind: "uncertain" },
  });
  assert.equal(failed.drafts.s, "hello");
  assert.deepEqual(failed.pending, {});
  const [, effects] = update(failed, { type: "retrySend" });
  assert.deepEqual(effects, [
    {
      type: "request",
      name: "sendMessage",
      sessionId: "s",
      body: { sessionId: "s", messageId: "client-uuid", content: "hello" },
    },
  ]);
});
test("message rows merge partial and complete assistant output and durable delivery changes", () => {
  const events: StoredEvent[] = [
    {
      sessionId: "s",
      seq: 1,
      createdAt: 1,
      kind: "message_appended",
      payload: {
        uuid: "u",
        role: "user",
        content: "hello",
        delivery_state: "queued",
      },
    },
    {
      sessionId: "s",
      seq: 2,
      createdAt: 1,
      kind: "delivery_changed",
      payload: { uuid: "u", delivery_state: "started", retry_count: null },
    },
    ...[
      {
        type: "stream_event",
        event: { type: "message_start", message: { id: "a" } },
      },
      {
        type: "stream_event",
        event: { type: "content_block_delta", delta: { text: "part" } },
      },
      {
        type: "assistant",
        message: { id: "a", content: [{ type: "text", text: "complete" }] },
      },
    ].map((payload, i) => ({
      sessionId: "s",
      seq: i + 3,
      createdAt: 100 + i,
      kind: "agent_event" as const,
      payload,
    })),
  ];
  // Each row keeps the time of the first event that created it, not of later updates.
  assert.deepEqual(chatRows(events), [
    { key: "u", role: "user", text: "hello", delivery: "started", at: 1 },
    { key: "a", role: "assistant", text: "complete", at: 100 },
  ]);
});

test("tool calls and results start collapsed and retain expansion during updates", async () => {
  const { dom, handle } = await fixture();
  const emit = (
    seq: number,
    payload: Extract<StoredEvent, { kind: "agent_event" }>["payload"],
  ) =>
    handle.dispatch({
      type: "event",
      event: {
        kind: "agent_event",
        sessionId: "s",
        seq,
        createdAt: 1,
        payload,
      },
    });
  emit(1, {
    type: "stream_event",
    event: { type: "message_start", message: { id: "a" } },
  });
  emit(2, {
    type: "assistant",
    message: {
      id: "a",
      content: [
        { type: "text", text: "Checking the files." },
        {
          type: "tool_use",
          id: "t1",
          name: "Read",
          input: { path: "/work/file" },
        },
      ],
    },
  });
  emit(3, {
    type: "user",
    message: {
      content: [
        { type: "tool_result", tool_use_id: "t1", content: "file contents" },
      ],
    },
  });
  await handle.settle();
  const details = [
    ...dom.window.document.querySelectorAll<HTMLDetailsElement>(
      "details.tool-details",
    ),
  ];
  assert.equal(details.length, 2);
  assert.deepEqual(
    details.map((item) => item.querySelector("summary")?.textContent),
    ["Read call", "Read result"],
  );
  assert.ok(details.every((item) => !item.open));
  assert.equal(
    dom.window.document
      .querySelector(".message.assistant > .markdown")
      ?.textContent?.trim(),
    "Checking the files.",
  );
  details[0]!.open = true;
  emit(4, {
    type: "assistant",
    message: { id: "b", content: [{ type: "text", text: "Done." }] },
  });
  await handle.settle();
  const expanded = dom.window.document.querySelector<HTMLDetailsElement>(
    "details.tool-details",
  );
  assert.equal(expanded, details[0]);
  assert.equal(expanded?.open, true);
  assert.match(
    expanded?.querySelector("pre")?.textContent ?? "",
    /\/work\/file/,
  );
  details[0]!.open = false;
  assert.equal(details[0]!.open, false);
  handle.stop();
  dom.window.close();
});

test("a tool group stays expanded as streamed results and calls arrive", async () => {
  const { dom, handle } = await fixture();
  const emit = (
    seq: number,
    content: import("../src/generated/contract.ts").JsonValue[],
    type = "assistant",
  ) =>
    handle.dispatch({
      type: "event",
      event: {
        sessionId: "s",
        seq,
        createdAt: 1,
        kind: "agent_event",
        payload: { type, message: { id: `group-${seq}`, content } },
      },
    });
  emit(1, [{ type: "tool_use", id: "t1", name: "Read", input: { path: "a" } }]);
  await handle.settle();
  const group =
    dom.window.document.querySelector<HTMLDetailsElement>(".tool-group")!;
  assert.ok(group);
  group.open = true;
  emit(
    2,
    [{ type: "tool_result", tool_use_id: "t1", content: "found" }],
    "user",
  );
  emit(3, [{ type: "tool_use", id: "t2", name: "Bash", input: { cmd: "ls" } }]);
  await handle.settle();
  assert.equal(dom.window.document.querySelector(".tool-group"), group);
  assert.equal(group.open, true);
  assert.match(group.querySelector("summary")!.textContent!, /2 tool calls/);
  emit(
    4,
    [
      {
        type: "tool_result",
        tool_use_id: "t2",
        content: "failed",
        is_error: true,
      },
    ],
    "user",
  );
  await handle.settle();
  assert.equal(dom.window.document.querySelectorAll(".tool-group").length, 1);
  assert.match(
    dom.window.document.querySelector(".tool-error > summary")!.textContent!,
    /error/,
  );
  assert.ok(!dom.window.document.querySelector(".tool-group .tool-error"));
  handle.stop();
});

test("Stop is disabled when idle and remains available for unacknowledged work", async () => {
  const { dom, handle } = await fixture();
  const stop =
    dom.window.document.querySelector<HTMLButtonElement>("button.stop")!;
  assert.equal(stop.disabled, false);
  assert.equal(
    dom.window.document.querySelector(".toolbar .activity-indicator"),
    null,
  );
  for (const [seq, changes, disabled] of [
    [1, { turnActive: false, running: true }, true],
    [2, { turnActive: false, dispatching: 1 }, false],
    [3, { turnActive: false, queued: 1, queueHeld: false }, false],
    [4, { turnActive: false, queued: 1, queueHeld: true }, true],
    [5, { turnActive: true, stopping: true }, true],
  ] as const) {
    handle.dispatch({
      type: "event",
      event: {
        sessionId: "s",
        seq,
        createdAt: 1,
        kind: "state_summary",
        payload: { ...session.status, ...changes },
      },
    });
    await handle.settle();
    assert.equal(stop.disabled, disabled);
  }
  handle.stop();
});

test("peer messages start folded by origin and retain manual expansion across arrivals", async () => {
  const { dom, handle } = await fixture();
  for (const [seq, sender] of [
    [1, "peer-session"],
    [2, undefined],
  ] as const) {
    handle.dispatch({
      type: "event",
      event: {
        sessionId: "s",
        seq,
        createdAt: 1,
        kind: "message_appended",
        payload: {
          uuid: `origin-${seq}`,
          role: "user",
          content: "Message from boblm session peer-session:\n\nHello",
          delivery_state: "started",
          ...(sender ? { sender_session_id: sender } : {}),
        },
      },
    });
  }
  await handle.settle();
  assert.equal(
    dom.window.document.querySelectorAll(".message.user.peer").length,
    1,
  );
  assert.equal(
    dom.window.document.querySelectorAll(".message.user:not(.peer)").length,
    1,
  );
  const doc = dom.window.document;
  const peer = doc.querySelector<HTMLElement>(".message.peer")!;
  const owner = doc.querySelector<HTMLElement>(".message.user:not(.peer)")!;
  assert.ok(peer.classList.contains("message-collapsed"));
  assert.ok(peer.querySelector(".message-preview"));
  assert.equal(owner.classList.contains("message-collapsed"), false);
  peer
    .querySelector<HTMLElement>("button[aria-label='Expand message']")!
    .click();
  await handle.settle();
  assert.equal(peer.classList.contains("message-collapsed"), false);
  assert.equal(peer.querySelector(".message-preview"), null);
  handle.dispatch({
    type: "event",
    event: {
      sessionId: "s",
      seq: 3,
      createdAt: 2,
      kind: "message_appended",
      payload: {
        uuid: "origin-3",
        role: "user",
        content: "Another long peer message",
        delivery_state: "started",
        sender_session_id: "another-peer",
      },
    },
  });
  await handle.settle();
  assert.equal(
    peer.classList.contains("message-collapsed"),
    false,
    "new input must not refold an expanded peer",
  );
  assert.equal(
    doc.querySelectorAll(".message.peer.message-collapsed").length,
    1,
  );
  peer
    .querySelector<HTMLElement>("button[aria-label='Collapse message']")!
    .click();
  await handle.settle();
  assert.ok(peer.classList.contains("message-collapsed"));
  handle.stop();
});

test("prompt suggestions show only in empty idle composer and Tab accepts a draft", async () => {
  const { dom, handle } = await fixture();
  const doc = dom.window.document;
  const field = doc.querySelector<HTMLTextAreaElement>("#composer")!;
  const overlay = () => doc.querySelector<HTMLElement>(".composer-suggestion");
  const summary = (
    seq: number,
    promptSuggestion?: string,
    turnActive = false,
  ) => {
    handle.dispatch({
      type: "event",
      event: {
        kind: "state_summary",
        sessionId: "s",
        seq,
        createdAt: 1,
        payload: {
          ...session.status,
          turnActive,
          promptSuggestion: promptSuggestion ?? null,
        },
      },
    });
  };
  summary(1, "Explain the example");
  await handle.settle();
  assert.equal(field.hasAttribute("placeholder"), false);
  assert.equal(field.placeholder, "");
  assert.equal(overlay()?.textContent, "Explain the example");
  assert.equal(
    field.getAttribute("aria-describedby"),
    "composer-suggestion-hint",
  );
  assert.equal(
    doc.querySelector("#composer-suggestion-hint")?.textContent,
    "Explain the example",
  );
  handle.dispatch({
    type: "listEvent",
    event: {
      kind: "state_summary",
      sessionId: "s",
      seq: 1,
      createdAt: 1,
      payload: { ...session.status, turnActive: false },
    },
  });
  await handle.settle();
  assert.equal(overlay()?.textContent, "Explain the example");
  assert.equal(field.value, "");
  const tab = new dom.window.KeyboardEvent("keydown", {
    key: "Tab",
    bubbles: true,
    cancelable: true,
  });
  field.dispatchEvent(tab);
  await handle.settle();
  assert.equal(tab.defaultPrevented, true);
  assert.equal(field.value, "Explain the example");
  assert.equal(field.selectionStart, "Explain the example".length);
  assert.equal(handle.model().drafts.s, "Explain the example");
  assert.equal(field.hasAttribute("placeholder"), false);
  assert.equal(overlay(), null);
  assert.equal(field.getAttribute("aria-describedby"), null);
  field.value = "My own prompt";
  field.dispatchEvent(new dom.window.Event("input", { bubbles: true }));
  await handle.settle();
  assert.equal(overlay(), null);
  field.value = "";
  field.dispatchEvent(new dom.window.Event("input", { bubbles: true }));
  await handle.settle();
  assert.equal(overlay()?.textContent, "Explain the example");
  summary(2, undefined, true);
  await handle.settle();
  assert.equal(overlay(), null);
  const plainTab = new dom.window.KeyboardEvent("keydown", {
    key: "Tab",
    bubbles: true,
    cancelable: true,
  });
  field.dispatchEvent(plainTab);
  assert.equal(plainTab.defaultPrevented, false);
  summary(3);
  await handle.settle();
  field.dispatchEvent(
    new dom.window.KeyboardEvent("keydown", { key: "Tab", bubbles: true }),
  );
  assert.equal(field.value, "");
});
