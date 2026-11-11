import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { mountProgram } from "@pairshaped/hypertea/testing";
import { initial, program, update, type Model } from "../src/app.ts";
import type {
  ModelOption,
  ProjectInfo,
  SessionInfo,
} from "../src/generated/contract.ts";

const P1 = "20000000-0000-4000-8000-000000000001";
const P2 = "20000000-0000-4000-8000-000000000002";
const models: ModelOption[] = [
  {
    model: "sonnet",
    label: "sonnet",
    agent: "claude",
    efforts: ["low", "medium", "high"],
  },
  {
    model: "opus",
    label: "opus",
    agent: "claude",
    efforts: ["low", "medium", "high"],
  },
  {
    model: "gpt-6-luna",
    label: "luna",
    agent: "codex",
    efforts: ["low", "high"],
  },
  {
    model: "gpt-6.1-sol",
    label: "sol",
    agent: "codex",
    efforts: ["low", "high", "max"],
  },
];
const project = (id: string, name: string, path: string): ProjectInfo => ({
  projectId: id,
  name,
  path,
  createdAt: 1,
});
const chat = (
  id: string,
  projectId: string,
  agent: "claude" | "codex",
): SessionInfo => ({
  name: "",
  sessionId: id,
  projectId,
  agent,
  workingDirectory: "/work",
  agentSessionId: null,
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
    model: agent === "codex" ? "gpt-6-luna" : "sonnet",
    effort: null,
  },
});
const base: Model = {
  ...initial("t"),
  models,
  projects: [
    project(P1, "app", "/work/app"),
    project(P2, "docs", "/work/docs"),
  ],
  sessions: [chat("c1", P1, "claude")],
};

test("adding a project shows the server's reply or its reason", () => {
  const [added] = update(base, {
    type: "reply",
    name: "addProject",
    sessionId: P2,
    outcome: { kind: "replied", reply: project("p3", "new", "/work/new") },
  });
  assert.equal(added.projects.at(-1)?.path, "/work/new");
  const [refused] = update(base, {
    type: "reply",
    name: "addProject",
    sessionId: P2,
    outcome: {
      kind: "rejected",
      error: {
        code: "invalid_request",
        message: '"/nope" is not an existing folder on this machine.',
      },
    },
  });
  assert.match(refused.error, /not an existing folder/);
});

test("a project is removed by the server's say-so and only appears removable when it has no chats", () => {
  const [, effects] = update(base, { type: "removeProject", projectId: P2 });
  assert.deepEqual(effects, [
    {
      type: "request",
      name: "removeProject",
      sessionId: P2,
      body: { projectId: P2 },
    },
  ]);
  const [gone] = update(base, {
    type: "reply",
    name: "removeProject",
    sessionId: P2,
    outcome: { kind: "replied", reply: {} },
  });
  assert.deepEqual(
    gone.projects.map((p) => p.projectId),
    [P1],
  );
  const [kept] = update(base, {
    type: "reply",
    name: "removeProject",
    sessionId: P1,
    outcome: {
      kind: "rejected",
      error: {
        code: "project_in_use",
        message: "This project still has 1 chat",
      },
    },
  });
  assert.equal(kept.projects.length, 2);
  assert.match(kept.error, /still has 1 chat/);
});

test("the New chat dialog opens on a project with Opus high, and picking a model resets an invalid effort", () => {
  const [open] = update(base, { type: "openNewChat", projectId: P1 });
  assert.deepEqual(open.newChat, {
    name: "",
    projectId: P1,
    model: "opus",
    effort: "high",
  });
  const remembered = {
    ...base,
    lastChoice: { model: "gpt-6.1-sol", effort: "max" },
  };
  assert.deepEqual(
    update(remembered, { type: "openNewChat", projectId: P2 })[0].newChat,
    {
      name: "",
      projectId: P2,
      model: "opus",
      effort: "high",
    },
  );
  // Previous choices do not change the default for a new chat.
  const stale = { ...base, lastChoice: { model: "retired", effort: "low" } };
  assert.deepEqual(
    update(stale, { type: "openNewChat", projectId: P1 })[0].newChat,
    {
      name: "",
      projectId: P1,
      model: "opus",
      effort: "high",
    },
  );
  const [picked] = update(remembered, { type: "openNewChat", projectId: P1 });
  const [sol] = update(picked, { type: "newChatModel", model: "gpt-6.1-sol" });
  const [max] = update(sol, { type: "newChatEffort", effort: "max" });
  const [codexLuna] = update(max, {
    type: "newChatModel",
    model: "gpt-6-luna",
  });
  assert.deepEqual(
    codexLuna.newChat,
    { name: "", projectId: P1, model: "gpt-6-luna", effort: null },
    "max is not available on luna",
  );
  const [withEffort] = update(codexLuna, {
    type: "newChatEffort",
    effort: "high",
  });
  assert.equal(withEffort.newChat?.effort, "high");
  assert.equal(
    update(withEffort, { type: "newChatModel", model: "opus" })[0].newChat
      ?.effort,
    "high",
    "an effort both models accept is kept",
  );
  assert.equal(update(withEffort, { type: "closeNewChat" })[0].newChat, null);
});

test("creating the chat sends the project and model, closes the dialog and remembers the choice", () => {
  const [open] = update(base, { type: "openNewChat", projectId: P1 });
  const [chosen] = update(open, { type: "newChatModel", model: "gpt-6-luna" });
  const [ready] = update(chosen, { type: "newChatEffort", effort: "low" });
  const [created, effects] = update(ready, {
    type: "createChat",
    id: "new-chat",
  });
  assert.equal(created.newChat, null);
  assert.deepEqual(effects, [
    { type: "persistChoice", choice: { model: "gpt-6-luna", effort: "low" } },
    {
      type: "request",
      name: "createSession",
      sessionId: "new-chat",
      body: {
        sessionId: "new-chat",
        name: "",
        projectId: P1,
        model: "gpt-6-luna",
        effort: "low",
      },
    },
  ]);
  assert.deepEqual(
    update(base, { type: "createChat", id: "x" })[1],
    [],
    "no dialog, nothing to create",
  );
  const [done] = update(created, {
    type: "reply",
    name: "createSession",
    sessionId: "new-chat",
    outcome: { kind: "replied", reply: chat("new-chat", P1, "codex") },
  });
  assert.equal(done.active, "new-chat");
});

test("coming back to a tab asks for the list again, so projects changed elsewhere show up", () => {
  assert.deepEqual(update({ ...base, token: "t" }, { type: "refresh" })[1], [
    {
      type: "request",
      name: "listSessions",
      sessionId: "",
      body: {},
      listRequest: 1,
    },
  ]);
  assert.deepEqual(
    update(initial(""), { type: "refresh" })[1],
    [],
    "locked: nothing to ask",
  );
});

test("a list reply brings the projects with the chats", () => {
  const [next] = update(initial("t"), {
    type: "reply",
    name: "listSessions",
    sessionId: "",
    outcome: {
      kind: "replied",
      reply: {
        models,
        projects: [project(P1, "app", "/work/app")],
        sessions: [],
      },
    },
  });
  assert.equal(next.projects[0]?.name, "app");
  assert.equal(next.models.length, 4);
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

test("the sidebar groups chats under their project and offers New chat on each", async () => {
  const { dom, handle } = await mounted({ ...base, active: "c1" });
  const document = dom.window.document;
  const groups = [...document.querySelectorAll(".project")];
  assert.equal(groups.length, 2);
  assert.match(groups[0]!.textContent ?? "", /app/);
  assert.doesNotMatch(groups[0]!.textContent ?? "", /\/work\/app/);
  assert.doesNotMatch(
    groups[0]!.querySelector(".session")?.textContent ?? "",
    /0 queued/,
  );
  assert.equal(groups[0]!.querySelector("[data-new-chat]")?.textContent, "+");
  assert.equal(
    groups[0]!.querySelector("[data-new-chat]")?.getAttribute("aria-label"),
    "New chat in app",
  );
  assert.equal(groups[0]!.querySelectorAll(".session").length, 1);
  assert.equal(groups[1]!.querySelectorAll(".session").length, 0);
  assert.ok(groups[0]!.querySelector("[data-new-chat]"));
  assert.equal(
    groups[0]!.querySelector("[data-remove-project]"),
    null,
    "a project with chats cannot be removed",
  );
  assert.ok(groups[1]!.querySelector("[data-remove-project]"));
  const remove = groups[1]!.querySelector("[data-remove-project]")!;
  assert.equal(remove.textContent, "");
  assert.equal(remove.getAttribute("aria-label"), "Remove docs");
  assert.ok(remove.querySelector("svg"));
  assert.ok(remove.nextElementSibling?.hasAttribute("data-new-chat"));
  handle.stop();
  dom.window.close();
});

test("with no project the only way forward is adding one", async () => {
  const { dom, handle } = await mounted({ ...initial("t"), models });
  const document = dom.window.document;
  assert.equal(document.querySelectorAll("[data-new-chat]").length, 0);
  assert.match(
    document.querySelector("#messages")?.textContent ?? "",
    /Add a project/,
  );
  assert.equal(document.querySelector("#project-path"), null);
  assert.ok(document.querySelector(".sidebar header [data-browse-folders]"));
  handle.stop();
  dom.window.close();
});

test("the dialog groups models by agent, lists that model's efforts and closes on Escape", async () => {
  const { dom, handle } = await mounted({
    ...base,
    newChat: { name: "", projectId: P1, model: "gpt-6-luna", effort: null },
  });
  const document = dom.window.document;
  const dialog = document.querySelector('[role="dialog"]');
  assert.ok(dialog, "the dialog is open");
  assert.equal(dialog?.getAttribute("aria-modal"), "true");
  const groups = [...dialog!.querySelectorAll("optgroup")].map((g) => [
    g.label,
    g.querySelectorAll("option").length,
  ]);
  assert.deepEqual(groups, [
    ["Claude", 2],
    ["Codex", 2],
  ]);
  const model = dialog!.querySelector<HTMLSelectElement>(
    'select[aria-label="Model"]',
  )!;
  assert.equal(model.value, "gpt-6-luna");
  const efforts = [
    ...dialog!.querySelectorAll<HTMLOptionElement>(
      'select[aria-label="Reasoning effort"] option',
    ),
  ].map((o) => o.value);
  assert.deepEqual(
    efforts,
    ["", "low", "high"],
    "Default plus what luna accepts",
  );
  assert.match(
    dialog!.textContent ?? "",
    /app/,
    "it says which project the chat is for",
  );
  dialog!.dispatchEvent(
    new dom.window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
  );
  await handle.settle();
  assert.equal(document.querySelector('[role="dialog"]'), null);
  handle.stop();
  dom.window.close();
});

test("a chat's model picker offers only its own agent's models", async () => {
  const codex: Model = {
    ...base,
    sessions: [chat("c2", P1, "codex")],
    active: "c2",
  };
  const { dom, handle } = await mounted(codex);
  const options = [
    ...dom.window.document.querySelectorAll<HTMLOptionElement>(
      '.toolbar select[aria-label="Model"] option',
    ),
  ].map((o) => o.value);
  assert.deepEqual(options, ["gpt-6-luna", "gpt-6.1-sol"]);
  handle.stop();
  dom.window.close();
});

test("Pi models form their own group and offer only their supported efforts", async () => {
  const piModel = "deepseek-ai/DeepSeek-V4.1-Flash-1017";
  const { dom, handle } = await mounted({
    ...base,
    models: [
      ...models,
      {
        model: piModel,
        label: "dsf",
        agent: "pi",
        efforts: ["off", "minimal", "low", "medium", "high"],
      },
    ],
    newChat: { name: "", projectId: P1, model: piModel, effort: "low" },
  });
  const dialog = dom.window.document.querySelector('[role="dialog"]')!;
  const pi = dialog.querySelector('optgroup[label="Pi"]');
  assert.equal(pi?.querySelector("option")?.value, piModel);
  assert.equal(pi?.querySelector("option")?.textContent, "dsf");
  const effort = dialog.querySelector<HTMLSelectElement>(
    'select[aria-label="Reasoning effort"]',
  )!;
  assert.ok([...effort.options].some((o) => o.value === "off"));
  assert.ok(![...effort.options].some((o) => o.value === "xhigh"));
  handle.stop();
  dom.window.close();
});

test("Lock asks for confirmation before changing authentication", async () => {
  const { dom, handle } = await mounted(initial("fixture-token"));
  try {
    const lock = dom.window.document.querySelector<HTMLButtonElement>(
      'button[aria-label="Lock"]',
    )!;
    lock.click();
    await handle.settle();
    assert.equal(
      dom.window.document.querySelector('input[name="token"]'),
      null,
    );
    assert.deepEqual(handle.effects().at(-1)?.effect, { type: "confirmLock" });
  } finally {
    handle.stop();
    dom.window.close();
  }
});

test("the sidebar has no app heading and the chat heading is blank before selection", async () => {
  const { dom, handle } = await mounted(base);
  try {
    assert.equal(
      dom.window.document.querySelector(".sidebar header strong"),
      null,
    );
    assert.equal(
      dom.window.document.querySelector(".chat > header strong")?.textContent ??
        "",
      "",
    );
    handle.dispatch({ type: "select", id: "c1" });
    await handle.settle();
    assert.equal(
      dom.window.document.querySelector(".chat > header strong")?.textContent,
      "Untitled chat",
    );
  } finally {
    handle.stop();
    dom.window.close();
  }
});

test("chat naming fields share their placeholder and rename preserves input on failure", async () => {
  const [open] = update(base, { type: "openNewChat", projectId: P1 });
  const { dom, handle } = await mounted(open);
  const input = dom.window.document.querySelector<HTMLInputElement>(
    'input[aria-label="Chat name"]',
  )!;
  assert.equal(input.placeholder, "Name of chat...");
  assert.equal(input.value, "");
  assert.equal(
    dom.window.document.querySelectorAll('[role="dialog"] label').length,
    0,
  );
  handle.stop();
  dom.window.close();
  const [named] = update(open, { type: "newChatName", name: "  My work  " });
  const [, effects] = update(named, { type: "createChat", id: "new" });
  const request = effects.find((e) => e.type === "request");
  assert.equal(
    (request?.type === "request" ? (request.body as { name: string }) : null)
      ?.name,
    "My work",
  );
  const [rename] = update({ ...base, active: "c1" }, { type: "openRename" });
  const fixture = await mounted(rename);
  assert.equal(
    fixture.dom.window.document.querySelector<HTMLInputElement>(
      'input[aria-label="Chat name"]',
    )?.placeholder,
    "Name of chat...",
  );
  fixture.handle.stop();
  fixture.dom.window.close();
  const [edit] = update(rename, { type: "renameName", name: "New title" });
  const [saving] = update(edit, { type: "saveRename" });
  assert.equal(saving.renameChat?.saving, true);
  const [failed] = update(saving, {
    type: "reply",
    name: "renameSession",
    sessionId: "c1",
    outcome: { kind: "uncertain", reason: "lost reply" },
  });
  assert.equal(failed.renameChat?.name, "New title");
  assert.equal(failed.renameChat?.saving, false);
  assert.match(failed.renameChat?.error ?? "", /lost/);
});

test("archived chat actions place Delete before fullscreen and deletion removes local chat data", async () => {
  const archived = {
    ...base.sessions[0]!,
    status: { ...base.sessions[0]!.status, archived: true },
  };
  const model = {
    ...base,
    active: "c1",
    sessions: [archived],
    drafts: { c1: "draft" },
  };
  const { dom, handle } = await mounted(model);
  const buttons = [
    ...dom.window.document.querySelectorAll(".chat > header button"),
  ].map((b) => b.getAttribute("aria-label") || b.textContent);
  assert.deepEqual(buttons, [
    "Chats",
    "Rename chat",
    "Reopen",
    "Delete chat",
    "Enter fullscreen",
    "Open questions (0 unanswered)",
    "Open drawer",
  ]);
  handle.stop();
  dom.window.close();
  assert.equal(
    update({ ...base, active: "c1" }, { type: "requestDelete" })[1].length,
    0,
  );
  const [, effects] = update(model, { type: "requestDelete" });
  assert.equal(effects[0]?.type, "confirmDelete");
  const [deleted] = update(model, {
    type: "listEvent",
    event: {
      kind: "deleted",
      sessionId: "c1",
      seq: 9,
      createdAt: 1,
      payload: {},
    },
  });
  assert.equal(deleted.active, null);
  assert.equal(deleted.sessions.length, 0);
  assert.equal(deleted.drafts.c1, undefined);
});

test("fullscreen sits between Archive and the drawer toggle and reflects browser state", async () => {
  const [unchanged, effects] = update(base, { type: "toggleFullscreen" });
  assert.equal(unchanged.fullscreen, false);
  assert.deepEqual(effects, [{ type: "toggleFullscreen" }]);
  const [active] = update(base, { type: "fullscreenChanged", active: true });
  const { dom, handle } = await mounted({ ...active, active: "c1" });
  const button = dom.window.document.querySelector(".chat-fullscreen")!;
  assert.equal(button.getAttribute("aria-label"), "Exit fullscreen");
  assert.equal(
    button.nextElementSibling?.getAttribute("aria-label"),
    "Open questions (0 unanswered)",
  );
  assert.equal(
    button.previousElementSibling?.getAttribute("aria-label"),
    "Archive",
  );
  const [inactive] = update(active, {
    type: "fullscreenChanged",
    active: false,
  });
  assert.equal(inactive.fullscreen, false);
  handle.stop();
  dom.window.close();
});

test("project rename sends only its ID and label and preserves sidebar order", () => {
  const [, effects] = update(base, {
    type: "renameProject",
    projectId: P1,
    name: "  Label  ",
  });
  assert.deepEqual(effects, [
    {
      type: "request",
      name: "renameProject",
      body: { projectId: P1, name: "Label" },
      sessionId: P1,
    },
  ]);
  const [renamed] = update(base, {
    type: "reply",
    name: "renameProject",
    sessionId: P1,
    outcome: { kind: "replied", reply: { ...base.projects[0], name: "Label" } },
  });
  assert.deepEqual(
    renamed.projects.map((p) => p.projectId),
    base.projects.map((p) => p.projectId),
  );
  assert.equal(renamed.projects[0]?.path, base.projects[0]?.path);
  assert.deepEqual(renamed.sessions, base.sessions);
});

test("an archived chat keeps its composer and attachments available for a new prompt", async () => {
  const { dom, handle } = await mounted({
    ...base,
    active: "c1",
    sessions: [
      {
        ...base.sessions[0]!,
        status: { ...base.sessions[0]!.status, archived: true },
      },
    ],
  });
  assert.equal(
    dom.window.document.querySelector<HTMLTextAreaElement>("#composer")
      ?.readOnly,
    false,
  );
  assert.equal(
    dom.window.document.querySelector<HTMLButtonElement>(
      'button[aria-label="Attach"]',
    )?.disabled,
    false,
  );
  handle.stop();
  dom.window.close();
});

test("clicking the chat title opens its rename dialog without a separate action button", async () => {
  const { dom, handle } = await mounted({ ...base, active: "c1" });
  const doc = dom.window.document;
  const title = doc.querySelector<HTMLButtonElement>(
    ".chat > header strong .chat-title",
  )!;
  assert.equal(title.getAttribute("aria-label"), "Rename chat");
  assert.equal(
    doc.querySelector('.chat > header > button[aria-label="Rename"]'),
    null,
  );
  title.click();
  await handle.settle();
  assert.equal(handle.model().renameChat?.sessionId, "c1");
  assert.ok(doc.querySelector('input[aria-label="Chat name"]'));
  handle.stop();
  dom.window.close();
});

test("project carets precede names and follow open, closed and empty states", async () => {
  const { dom, handle } = await mounted({ ...base, active: "c1" });
  const doc = dom.window.document;
  const project = doc.querySelector<HTMLButtonElement>(
    `[data-toggle-project="${P1}"]`,
  )!;
  const empty = doc.querySelector<HTMLButtonElement>(
    `[data-toggle-project="${P2}"]`,
  )!;
  assert.equal(
    project.firstElementChild?.classList.contains("project-caret"),
    true,
  );
  assert.equal(project.getAttribute("aria-expanded"), "true");
  const open = project.querySelector("path")!.getAttribute("d");
  project.click();
  await handle.settle();
  assert.equal(project.getAttribute("aria-expanded"), "false");
  const closed = project.querySelector("path")!.getAttribute("d");
  assert.notEqual(closed, open);
  assert.equal(empty.getAttribute("aria-expanded"), "false");
  assert.equal(empty.querySelector("path")!.getAttribute("d"), closed);
  project.click();
  await handle.settle();
  assert.equal(project.querySelector("path")!.getAttribute("d"), open);
  handle.stop();
  dom.window.close();
});
