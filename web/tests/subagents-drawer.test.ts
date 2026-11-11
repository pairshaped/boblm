import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { mountProgram } from "@pairshaped/hypertea/testing";
import { initial, program, update, type Model } from "../src/app.ts";
import type {
  JsonValue,
  SessionInfo,
  StoredEvent,
} from "../src/generated/contract.ts";

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
const event = (
  session: string,
  seq: number,
  type: string,
  content: JsonValue[],
): StoredEvent => ({
  sessionId: session,
  seq,
  createdAt: 1_700_000_000_000 + seq * 1000,
  kind: "agent_event",
  payload: { type, message: { id: `m${seq}`, role: type, content } },
});
const call = (session: string, seq: number, id: string, description: string) =>
  event(session, seq, "assistant", [
    {
      type: "tool_use",
      id,
      name: "Agent",
      input: { description, subagent_type: "general-purpose" },
    },
  ]);
const result = (session: string, seq: number, id: string) =>
  event(session, seq, "user", [
    { type: "tool_result", tool_use_id: id, content: "ok" },
  ]);

// s1 has one running and one finished subagent; s2 has none.
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
  drawerTab: "agents",
  events: {
    s1: [
      call("s1", 1, "a", "Review cutover adoption"),
      result("s1", 2, "a"),
      call("s1", 3, "b", "Check the lock query cache"),
    ],
    s2: [],
  },
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

test("the drawer is closed by default, and the header toggle shows how many subagents are running", async () => {
  const { dom, handle } = await mounted(base);
  const doc = dom.window.document;
  assert.equal(doc.querySelector(".subagents"), null);
  const toggle = doc.querySelector("button.drawer-toggle");
  assert.ok(toggle);
  assert.equal(toggle.getAttribute("aria-expanded"), "false");
  assert.equal(toggle.querySelector("svg") !== null, true);
  handle.stop();
  dom.window.close();
});

test("toggling opens and closes the drawer; a fresh page starts closed", () => {
  const [open, effects] = update(base, { type: "toggleDrawer" });
  assert.equal(open.drawerOpen, true);
  assert.deepEqual(effects, []);
  const [shut] = update(open, { type: "toggleDrawer" });
  assert.equal(shut.drawerOpen, false);
  assert.equal(initial("t", {}, {}, {}, null, {}, {}, true).drawerOpen, true);
});

test("open, it lists Active and Done with counts, each subagent with an icon, a title and a time", async () => {
  const { dom, handle } = await mounted({ ...base, drawerOpen: true });
  const doc = dom.window.document;
  const drawer = doc.querySelector(".subagents");
  assert.ok(drawer);
  const headings = [...drawer.querySelectorAll("h3")].map((h) => h.textContent);
  assert.deepEqual(headings, ["Active · 1", "Done · 1"]);
  const items = [...drawer.querySelectorAll("li")];
  assert.equal(items.length, 2);
  assert.match(items[0]!.textContent ?? "", /Check the lock query cache/);
  assert.match(items[1]!.textContent ?? "", /Review cutover adoption/);
  for (const li of items) {
    assert.ok(li.querySelector("svg"), "an icon");
    assert.ok(li.querySelector("time"), "a time");
  }
  assert.equal(
    doc.querySelector("button.drawer-toggle")?.getAttribute("aria-expanded"),
    "true",
  );
  handle.stop();
  dom.window.close();
});

test("an icon is the same for the same subagent and differs between subagents", async () => {
  const { dom, handle } = await mounted({ ...base, drawerOpen: true });
  const icons = [
    ...dom.window.document.querySelectorAll(".subagents li svg"),
  ].map((s) => s.innerHTML);
  assert.equal(icons.length, 2);
  assert.notEqual(icons[0], icons[1]);
  handle.stop();
  dom.window.close();
  const again = await mounted({ ...base, drawerOpen: true });
  const second = [
    ...again.dom.window.document.querySelectorAll(".subagents li svg"),
  ].map((s) => s.innerHTML);
  assert.deepEqual(second, icons);
  again.handle.stop();
  again.dom.window.close();
});

test("the shared drawer stays usable with no agents and no questions", async () => {
  const { dom, handle } = await mounted({
    ...base,
    drawerOpen: true,
    active: "s2",
  });
  const doc = dom.window.document;
  assert.ok(doc.querySelector(".subagents"));
  assert.match(
    doc.querySelector('[role="tabpanel"]')!.textContent!,
    /No active subagents/,
  );
  assert.equal(
    doc.querySelector<HTMLButtonElement>(".drawer-toggle")!.disabled,
    false,
  );
  doc.querySelector<HTMLButtonElement>("#drawer-tab-jobs")!.click();
  await handle.settle();
  assert.equal(handle.model().drawerTab, "jobs");
  assert.match(
    doc.querySelector('[role="tabpanel"]')!.textContent!,
    /Background job status is unavailable/,
  );
  const mobileToggle = doc.querySelector<HTMLButtonElement>(
    ".subagents > header .mobile-drawer-toggle",
  )!;
  assert.ok(mobileToggle.querySelector("svg"));
  assert.equal(mobileToggle.getAttribute("aria-label"), "Close drawer");
  doc.querySelector<HTMLButtonElement>(".drawer-toggle")!.click();
  await handle.settle();
  assert.equal(doc.querySelector(".subagents"), null);
  assert.equal(handle.model().drawerOpen, false);
  handle.stop();
  dom.window.close();
});

test("tabs preserve question drafts and the selected tab survives closing and reopening", async () => {
  const { dom, handle } = await mounted({
    ...base,
    drawerOpen: true,
    questionDrafts: { q1: "My draft" },
  });
  const doc = dom.window.document;
  doc.querySelector<HTMLButtonElement>("#drawer-tab-jobs")!.click();
  await handle.settle();
  doc.querySelector<HTMLButtonElement>("#drawer-tab-agents")!.click();
  await handle.settle();
  assert.equal(handle.model().questionDrafts.q1, "My draft");
  doc.querySelector<HTMLButtonElement>(".drawer-toggle")!.click();
  await handle.settle();
  doc.querySelector<HTMLButtonElement>(".drawer-toggle")!.click();
  await handle.settle();
  assert.equal(handle.model().drawerTab, "agents");
  assert.equal(
    doc.querySelector("#drawer-tab-agents")!.getAttribute("aria-selected"),
    "true",
  );
  doc.querySelector<HTMLButtonElement>("#drawer-tab-agents")!.dispatchEvent(
    new dom.window.KeyboardEvent("keydown", {
      key: "ArrowLeft",
      bubbles: true,
    }),
  );
  await handle.settle();
  assert.equal(handle.model().drawerTab, "jobs");
  assert.equal(doc.activeElement?.id, "drawer-tab-jobs");
  handle.stop();
  dom.window.close();
});

test("similar ids still get clearly different icons", async () => {
  const many: StoredEvent[] = [];
  for (let i = 1; i <= 6; i++) many.push(call("s1", i, `t${i}`, `Agent ${i}`));
  const { dom, handle } = await mounted({
    ...base,
    drawerOpen: true,
    events: { s1: many, s2: [] },
  });
  const fills = [
    ...dom.window.document.querySelectorAll(
      ".subagents li svg ellipse:first-child",
    ),
  ].map((e) => e.getAttribute("fill"));
  assert.equal(fills.length, 6);
  assert.ok(new Set(fills).size >= 5, `hues too alike: ${fills.join(" ")}`);
  handle.stop();
  dom.window.close();
});

test("a failed subagent keeps its time in the same row", async () => {
  const failed = [
    call("s1", 1, "a", "Broke"),
    event("s1", 2, "user", [
      { type: "tool_result", tool_use_id: "a", content: "no", is_error: true },
    ]),
  ];
  const { dom, handle } = await mounted({
    ...base,
    drawerOpen: true,
    events: { s1: failed, s2: [] },
  });
  const li = dom.window.document.querySelector(".subagents li.failed");
  assert.ok(li);
  assert.ok(li.querySelector("small.failed"));
  assert.ok(li.querySelector("time"));
  assert.equal(
    dom.window.document
      .querySelector(".subagents")
      ?.textContent?.match(/failed/g)?.length,
    1,
  );
  handle.stop();
  dom.window.close();
});

test("running agents have animated dots, and clicking opens a read-only chat under the tabs", async () => {
  const { dom, handle } = await mounted({ ...base, drawerOpen: true });
  const doc = dom.window.document;
  assert.equal(
    doc.querySelectorAll(".subagents .activity-indicator.running span").length,
    3,
  );
  assert.equal(doc.querySelectorAll(".subagents kbd").length, 0);
  doc
    .querySelector<HTMLButtonElement>(
      '[aria-label="View Check the lock query cache"]',
    )!
    .click();
  await handle.settle();
  assert.equal(handle.model().subagentConversation?.key, "b");
  assert.equal(
    doc.querySelector(".subagent-titlebar strong")!.textContent,
    "Check the lock query cache",
  );
  assert.ok(doc.querySelector("#drawer-tab-jobs"));
  assert.ok(doc.querySelector("#drawer-tab-agents"));
  assert.equal(doc.querySelector(".subagent-conversation textarea"), null);
  doc
    .querySelector<HTMLButtonElement>('[aria-label="Back to subagents"]')!
    .click();
  await handle.settle();
  assert.equal(handle.model().subagentConversation, null);
  doc
    .querySelector<HTMLButtonElement>(
      '[aria-label="View Check the lock query cache"]',
    )!
    .click();
  await handle.settle();
  doc.querySelector<HTMLButtonElement>("#drawer-tab-jobs")!.click();
  await handle.settle();
  assert.equal(handle.model().drawerTab, "jobs");
  doc.querySelector<HTMLButtonElement>("#drawer-tab-agents")!.click();
  await handle.settle();
  assert.equal(handle.model().subagentConversation, null);
  handle.stop();
  dom.window.close();
});

test("subagent replies cannot affect the parent timeline or reopen a previous agent", () => {
  let [model] = update(
    { ...base, drawerOpen: true },
    { type: "openSubagent", key: "b" },
  );
  const view = model.subagentConversation!;
  const reply = {
    type: "subagentReply" as const,
    generation: view.generation,
    owner: "s1",
    key: "b",
    outcome: {
      kind: "replied" as const,
      reply: {
        sessionId: "s1",
        agentKey: "b",
        events: [
          event("s1", 100, "assistant", [
            { type: "text", text: "## Working\n\n- Inspect the cache" },
          ]),
        ],
        cursor: 200,
        hasMore: false,
      },
    },
  };
  [model] = update(model, reply);
  assert.equal(
    model.subagentConversation!.timeline.rows[0]!.text,
    "## Working\n\n- Inspect the cache",
  );
  assert.equal(model.events, base.events);
  assert.deepEqual(model.cursors, base.cursors);
  assert.equal(model.subagentConversation!.following, true);
  [model] = update(model, { type: "subagentFollowing", value: false });
  assert.equal(model.subagentConversation!.following, false);
  [model] = update(model, { type: "backToAgents" });
  assert.equal(update(model, reply)[0], model);
  [model] = update(model, { type: "openSubagent", key: "a" });
  assert.equal(update(model, reply)[0], model);
});

test("opening the new-chat dialog closes the phone project drawer", () => {
  const [opened] = update(
    { ...base, sidebarOpen: true },
    { type: "openNewChat", projectId: "p" },
  );
  assert.ok(opened.newChat);
  assert.equal(opened.sidebarOpen, false);
});

test("subagent tool names survive when a result arrives in a later transcript page", () => {
  let [model] = update(
    { ...base, drawerOpen: true },
    { type: "openSubagent", key: "b" },
  );
  const generation = model.subagentConversation!.generation;
  const reply = (events: StoredEvent[], cursor: number) => ({
    type: "subagentReply" as const,
    generation,
    owner: "s1",
    key: "b",
    outcome: {
      kind: "replied" as const,
      reply: { sessionId: "s1", agentKey: "b", events, cursor, hasMore: false },
    },
  });
  [model] = update(
    model,
    reply(
      [
        event("s1", 100, "assistant", [
          {
            type: "tool_use",
            id: "read-1",
            name: "Read",
            input: { path: "file.rs" },
          },
        ]),
      ],
      200,
    ),
  );
  [model] = update(
    model,
    reply(
      [
        event("s1", 300, "user", [
          { type: "tool_result", tool_use_id: "read-1", content: "source" },
        ]),
      ],
      400,
    ),
  );
  assert.equal(
    model.subagentConversation!.timeline.rows.at(-1)!.tools![0]!.name,
    "Read",
  );
});

test("subagents remain in the drawer after their launch messages leave the chat window", async () => {
  let model: Model = { ...base, drawerOpen: true };
  // A real chat opens on a recent snapshot, then advances beyond its retained window.
  [model] = update(model, { type: "select", id: "s2" });
  [model] = update(model, { type: "select", id: "s1" });
  const history = model.history.s1!;
  [model] = update(model, {
    type: "historyReply",
    sessionId: "s1",
    generation: history.generation,
    outcome: {
      kind: "replied",
      reply: {
        sessionId: "s1",
        partialId: null,
        status: chat("s1").status,
        cursor: 3,
        context: [],
        hasMore: false,
        events: base.events.s1!,
      },
    },
  });
  for (let seq = 4; seq < 1000; seq++) {
    [model] = update(model, {
      type: "event",
      event: event("s1", seq, "assistant", [
        { type: "text", text: `Later progress ${seq}` },
      ]),
    });
  }
  const { dom, handle } = await mounted(model);
  assert.ok(
    dom.window.document.querySelector(
      '[aria-label="View Check the lock query cache"]',
    ),
    "the running agent must survive window eviction",
  );
  assert.ok(
    dom.window.document.querySelector(
      '[aria-label="View Review cutover adoption"]',
    ),
    "the finished agent must survive window eviction",
  );
  handle.stop();
  dom.window.close();
});

test("a fresh latest snapshot restores agents whose lifecycle records are outside its page", () => {
  let [model] = update(base, { type: "select", id: "s2" });
  [model] = update(model, { type: "select", id: "s1" });
  [model] = update(model, {
    type: "historyReply",
    sessionId: "s1",
    generation: model.history.s1!.generation,
    outcome: {
      kind: "replied",
      reply: {
        sessionId: "s1",
        partialId: null,
        status: chat("s1").status,
        cursor: 1000,
        hasMore: true,
        context: base.events.s1!,
        events: [
          event("s1", 1000, "assistant", [
            { type: "text", text: "Recent conversation" },
          ]),
        ],
      },
    },
  });
  assert.deepEqual(
    model.subagents.map((agent) => [agent.key, agent.state]),
    [
      ["b", "active"],
      ["a", "done"],
    ],
  );
  [model] = update(model, { type: "following", value: false });
  model = {
    ...model,
    history: { s1: { ...model.history.s1!, hasNewer: true } },
  };
  [model] = update(model, { type: "event", event: result("s1", 1001, "b") });
  assert.equal(
    model.subagents.find((agent) => agent.key === "b")!.state,
    "done",
    "completion is tracked even while its result is outside the visible window",
  );
  [model] = update(model, { type: "history", direction: "older" });
  [model] = update(model, {
    type: "historyReply",
    sessionId: "s1",
    generation: model.history.s1!.generation,
    outcome: {
      kind: "replied",
      reply: {
        sessionId: "s1",
        partialId: null,
        status: chat("s1").status,
        cursor: 1000,
        hasMore: false,
        context: base.events.s1!,
        events: [],
      },
    },
  });
  assert.equal(
    model.subagents.find((agent) => agent.key === "b")!.state,
    "done",
    "a stale snapshot must not revive an agent",
  );
  [model] = update(model, { type: "select", id: "s2" });
  assert.deepEqual(model.subagents, []);
});

test("jobs survive empty history, keep completed agents waiting, and cap visible rows", async () => {
  const sessions = base.sessions.map((s) =>
    s.sessionId !== "s1"
      ? s
      : {
          ...s,
          status: {
            ...s.status,
            jobs: {
              available: true,
              tasks: Array.from({ length: 75 }, (_, i) => ({
                id: `job-${i}`,
                command: `cargo test ${i}`,
                agentKey: i === 0 ? "a" : null,
              })),
            },
          },
        },
  );
  const { dom, handle } = await mounted({
    ...base,
    sessions,
    events: {},
    history: {},
    drawerOpen: true,
    subagents: [
      {
        key: "a",
        title: "Worker",
        state: "done",
        startedAt: 1,
        endedAt: 2,
        revision: 2,
      },
    ],
  });
  const doc = dom.window.document;
  assert.match(
    doc.querySelector(".subagents-body")!.textContent!,
    /Waiting for jobs/,
  );
  assert.equal(
    doc.querySelector("li.waiting time")!.getAttribute("title"),
    "Started",
  );
  assert.deepEqual(
    [...doc.querySelectorAll('[role="tab"]')].map((t) => t.id),
    ["drawer-tab-agents", "drawer-tab-jobs"],
  );
  doc.querySelector<HTMLButtonElement>("#drawer-tab-jobs")!.click();
  await handle.settle();
  assert.equal(doc.querySelectorAll(".jobs-body li").length, 50);
  assert.match(
    doc.querySelector(".jobs-body")!.textContent!,
    /Showing 50 of 75 running jobs/,
  );
  assert.match(doc.querySelector(".jobs-body")!.textContent!, /Worker/);
  assert.match(doc.querySelector(".jobs-body")!.textContent!, /This chat/);
  handle.stop();
  dom.window.close();
});

const boblmChild = (): SessionInfo => ({
  ...chat("child"),
  name: "Worker",
  parentSessionId: "s1",
  rootSessionId: "s1",
  role: "implementer",
  status: {
    ...chat("child").status,
    running: true,
    turnActive: true,
    model: "gpt-6.1-sol",
    effort: "high",
  },
});
const boblmBase = (): Model => ({
  ...base,
  sessions: [chat("s1"), boblmChild()],
  events: { s1: [], child: [] },
  drawerOpen: true,
});
const childEvent = (seq: number, text: string): StoredEvent => ({
  sessionId: "child",
  seq,
  createdAt: 1_700_000_000_000 + seq * 1000,
  kind: "agent_event",
  payload: {
    type: "assistant",
    message: { id: `c${seq}`, content: [{ type: "text", text }] },
  },
});
function loadedChild(model: Model): Model {
  const [opened, effects] = update(model, {
    type: "openSubagent",
    key: "child",
  });
  assert.equal(opened.subagentConversation?.boblm, true);
  assert.ok(
    effects.some(
      (effect) =>
        effect.type === "historyRequest" &&
        effect.sessionId === "child" &&
        effect.direction === "latest",
    ),
  );
  const history = opened.history.child!;
  const [replied] = update(opened, {
    type: "historyReply",
    sessionId: "child",
    generation: history.generation,
    outcome: {
      kind: "replied",
      reply: {
        sessionId: "child",
        cursor: 1,
        status: boblmChild().status,
        events: [childEvent(1, "child says hi")],
        context: [],
        hasMore: true,
        partialId: null,
      },
    },
  });
  return replied;
}

test("a boblm child opens from the session list into its own history, not a transcript poll", () => {
  const opened = loadedChild(boblmBase());
  assert.equal(opened.subagentConversation?.key, "child");
  assert.equal(opened.subagentConversation?.boblm, true);
  assert.equal(opened.history.child?.loaded, true);
  assert.match(
    opened.history.child!.timeline.rows.map((row) => row.text).join(" "),
    /child says hi/,
  );
  const [live] = update(opened, {
    type: "event",
    event: childEvent(2, "live"),
  });
  assert.match(
    live.history.child!.timeline.rows.map((row) => row.text).join(" "),
    /child says hi live/,
  );
  const [paged, effects] = update(live, {
    type: "subagentPage",
    latest: false,
  });
  assert.equal(paged.history.child?.direction, "older");
  assert.ok(
    effects.some(
      (effect) =>
        effect.type === "historyRequest" && effect.sessionId === "child",
    ),
  );
});

test("the boblm child header shows its route and status with Stop and Archive, and no composer", async () => {
  const model = loadedChild(boblmBase());
  const { dom, handle } = await mounted(model);
  const doc = dom.window.document;
  const header = doc.querySelector(".subagent-titlebar");
  assert.ok(header);
  assert.match(header.textContent ?? "", /Worker/);
  assert.match(header.textContent ?? "", /implementer/);
  assert.match(header.textContent ?? "", /gpt-6.1-sol/);
  assert.match(header.textContent ?? "", /high/);
  assert.match(header.textContent ?? "", /working/);
  assert.ok(header.querySelector('button[aria-label="Stop"]'));
  assert.ok(header.querySelector('button[aria-label="Archive"]'));
  assert.equal(doc.querySelector(".subagent-conversation textarea"), null);
  assert.match(
    doc.querySelector("#subagent-messages")?.textContent ?? "",
    /child says hi/,
  );
  const [stopped, effects] = update(model, {
    type: "commandOn",
    name: "stopSession",
    sessionId: "child",
  });
  assert.equal(stopped.active, "s1");
  assert.deepEqual(effects, [
    {
      type: "request",
      name: "stopSession",
      body: { sessionId: "child" },
      sessionId: "child",
    },
  ]);
  handle.stop();
  dom.window.close();
});

test("opening, updating and paging a boblm child leaves the parent's timeline, cursor and events alone", () => {
  let [parent] = update(boblmBase(), { type: "history", direction: "latest" });
  const parentEvent = event("s1", 5, "assistant", [
    { type: "text", text: "parent says hi" },
  ]);
  [parent] = update(parent, {
    type: "historyReply",
    sessionId: "s1",
    generation: parent.history.s1!.generation,
    outcome: {
      kind: "replied",
      reply: {
        sessionId: "s1",
        cursor: 5,
        status: chat("s1").status,
        events: [parentEvent],
        context: [],
        hasMore: false,
        partialId: null,
      },
    },
  });
  const before = {
    history: parent.history.s1,
    cursor: parent.cursors.s1,
    events: parent.events.s1,
  };
  assert.match(
    before.history!.timeline.rows.map((row) => row.text).join(" "),
    /parent says hi/,
  );
  const check = (model: Model, step: string) => {
    assert.deepEqual(model.history.s1, before.history, `${step}: timeline`);
    assert.equal(model.cursors.s1, before.cursor, `${step}: cursor`);
    assert.deepEqual(model.events.s1, before.events, `${step}: events`);
  };
  const [opening] = update(parent, { type: "openSubagent", key: "child" });
  check(opening, "opening");
  const loaded = loadedChild(parent);
  check(loaded, "loaded");
  const [live] = update(loaded, {
    type: "event",
    event: childEvent(2, "live"),
  });
  check(live, "live update");
  const [paging] = update(live, { type: "subagentPage", latest: false });
  check(paging, "paging");
  assert.equal(loaded.active, "s1");
});

test("an event for the parent keeps the open child's history and events, and the reverse", () => {
  let [parent] = update(boblmBase(), { type: "history", direction: "latest" });
  [parent] = update(parent, {
    type: "historyReply",
    sessionId: "s1",
    generation: parent.history.s1!.generation,
    outcome: {
      kind: "replied",
      reply: {
        sessionId: "s1",
        cursor: 5,
        status: chat("s1").status,
        events: [
          event("s1", 5, "assistant", [
            { type: "text", text: "parent says hi" },
          ]),
        ],
        context: [],
        hasMore: false,
        partialId: null,
      },
    },
  });
  const loaded = loadedChild(parent);
  const text = (model: Model, id: string) =>
    model.history[id]?.timeline.rows.map((row) => row.text).join(" ") ?? "";
  const [afterParent] = update(loaded, {
    type: "event",
    event: event("s1", 6, "assistant", [
      { type: "text", text: "parent again" },
    ]),
  });
  assert.match(text(afterParent, "s1"), /parent again/);
  assert.match(text(afterParent, "child"), /child says hi/);
  assert.deepEqual(afterParent.events.child, loaded.events.child);
  const [afterChild] = update(afterParent, {
    type: "event",
    event: childEvent(2, "live"),
  });
  assert.match(text(afterChild, "child"), /child says hi live/);
  assert.match(text(afterChild, "s1"), /parent again/);
  const [bottomed] = update(afterChild, { type: "bottom" });
  assert.match(text(bottomed, "child"), /child says hi live/);
});
