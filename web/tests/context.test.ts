import { emptyTimeline, projectEvents } from "../src/timeline.ts";
import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { mountProgram } from "@pairshaped/hypertea/testing";
import { initial, program, chatRows, type Model } from "../src/app.ts";
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
const message = (seq: number, uuid: string, content: string): StoredEvent => ({
  sessionId: "s",
  seq,
  createdAt: 1,
  kind: "message_appended",
  payload: { uuid, role: "user", content, delivery_state: "queued" },
});
const events: StoredEvent[] = [
  message(1, "m1", "remember BANANA"),
  {
    sessionId: "s",
    seq: 2,
    createdAt: 1,
    kind: "agent_session_changed",
    payload: { agent_session_id: "one", previous: null },
  },
  message(3, "m2", "/compact"),
  {
    sessionId: "s",
    seq: 4,
    createdAt: 1,
    kind: "agent_event",
    payload: { type: "system", subtype: "compact_boundary", session_id: "one" },
  },
  {
    sessionId: "s",
    seq: 5,
    createdAt: 1,
    kind: "agent_event",
    payload: { type: "system", subtype: "init", session_id: "one" },
  },
  message(6, "m3", "/clear"),
  {
    sessionId: "s",
    seq: 7,
    createdAt: 1,
    kind: "agent_session_changed",
    payload: { agent_session_id: "two", previous: "one" },
  },
  message(8, "m4", "what was the word?"),
];

test("compaction and a cleared conversation appear as markers between messages", () => {
  const rows = chatRows(events);
  assert.deepEqual(
    rows.map((r) => [r.role, r.text.slice(0, 22)]),
    [
      ["user", "remember BANANA"],
      ["user", "/compact"],
      ["marker", "Conversation compacted"],
      ["user", "/clear"],
      ["marker", "New conversation. Earl"],
      ["user", "what was the word?"],
    ],
    "the first session ID and ordinary init events add no marker",
  );
  assert.match(rows[4]!.text, /no longer sees/);
});

test("a Codex chat's markers and composer name Codex, not Claude", () => {
  const rows = chatRows(events, "codex");
  assert.match(rows[2]!.text, /^Conversation compacted\. Codex keeps/);
  assert.match(rows[4]!.text, /Codex no longer sees/);
  assert.doesNotMatch(rows.map((r) => r.text).join(" "), /Claude/);
});

test("markers render as separators, not messages", async () => {
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
  const handle = mountProgram({
    flags: model,
    node: dom.window.document.querySelector<HTMLElement>("#app")!,
    program,
  });
  await handle.settle();
  const document = dom.window.document;
  const markers = document.querySelectorAll('#messages [role="separator"]');
  assert.equal(markers.length, 2);
  assert.match(markers[0]!.textContent ?? "", /Conversation compacted/);
  assert.match(markers[1]!.textContent ?? "", /New conversation/);
  assert.equal(
    document.querySelectorAll("#messages article.message").length,
    4,
  );
  assert.equal(document.querySelectorAll("#messages article.marker").length, 0);
  const summary =
    "This session is being continued from a previous conversation that ran out of context. The summary below covers the earlier portion of the conversation.\n\nSummary: Keep working.";
  handle.dispatch({
    type: "event",
    event: {
      sessionId: "s",
      seq: 9,
      createdAt: 2,
      kind: "agent_event",
      payload: {
        type: "user",
        message: { content: [{ type: "text", text: summary }] },
      },
    },
  });
  await handle.settle();
  const detail = document.querySelector<HTMLDetailsElement>(
    "#messages details.tool-details",
  )!;
  assert.ok(detail);
  assert.equal(detail.open, false);
  assert.equal(
    detail.querySelector("summary")?.textContent,
    "Compaction summary",
  );
  assert.equal(detail.querySelector("pre")?.textContent, summary);
  detail.querySelector("summary")!.click();
  assert.equal(detail.open, true);
  handle.stop();
  dom.window.close();
});

test("compaction notices update in place for every agent and accept completion-only history", () => {
  for (const agent of ["Claude", "Codex", "Pi"]) {
    const event = (seq: number, subtype: string): StoredEvent => ({
      sessionId: "s",
      seq,
      kind: "agent_event",
      createdAt: seq,
      payload: { type: "system", subtype, compaction_id: "one" },
    });
    const started = projectEvents(
      emptyTimeline(),
      [event(1, "compact_started")],
      agent,
    );
    assert.match(started.rows[0]!.text, /Compaction started/);
    const completed = projectEvents(
      started,
      [event(2, "compact_boundary")],
      agent,
    ).rows;
    assert.equal(completed.length, 1);
    assert.equal(completed[0]!.seq, 1);
    assert.equal(completed[0]!.key, "compaction:one");
    assert.match(completed[0]!.text, /Conversation compacted/);
    assert.equal(
      projectEvents(emptyTimeline(), [event(2, "compact_boundary")], agent).rows
        .length,
      1,
    );
  }
});
