import { test } from "node:test";
import assert from "node:assert/strict";
import { initial, update } from "../src/app.ts";

const listing = {
  path: "/home/me",
  parent: null,
  directories: [{ name: "projects", path: "/home/me/projects" }],
};
test("folder picker starts at home and Use this folder adds the selected project once", () => {
  const [opened, effects] = update(initial("fixture"), {
    type: "openDirectories",
  });
  assert.equal(opened.directoryPicker?.loading, true);
  assert.deepEqual(effects, [
    {
      type: "request",
      name: "listDirectories",
      body: {},
      sessionId: "directory:1",
    },
    { type: "directoryFocus", open: true },
  ]);
  const [loaded] = update(opened, {
    type: "reply",
    name: "listDirectories",
    sessionId: "directory:1",
    outcome: { kind: "replied", reply: listing },
  });
  assert.deepEqual(loaded.directoryPicker?.listing, listing);
  const [selected, fill] = update(loaded, {
    type: "chooseDirectory",
    id: "project-request",
  });
  assert.equal(selected.directoryPicker, null);
  assert.deepEqual(fill, [
    { type: "directoryFocus", open: false },
    {
      type: "request",
      name: "addProject",
      body: { projectId: "project-request", path: "/home/me" },
      sessionId: "project-request",
    },
  ]);
  assert.deepEqual(
    update(selected, { type: "chooseDirectory", id: "second" })[1],
    [],
  );
});

test("folder navigation keeps the last selection on failure and ignores replies after cancel, reopen and lock", () => {
  let [model] = update(initial("fixture"), { type: "openDirectories" });
  const reply = (sessionId: string) =>
    ({
      type: "reply",
      name: "listDirectories",
      sessionId,
      outcome: { kind: "replied", reply: listing },
    }) as const;
  [model] = update(model, reply("directory:1"));
  let effects;
  [model, effects] = update(model, {
    type: "browseDirectory",
    path: "/home/me/projects",
  });
  assert.deepEqual(effects, [
    {
      type: "request",
      name: "listDirectories",
      body: { path: "/home/me/projects" },
      sessionId: "directory:2",
    },
  ]);
  assert.deepEqual(
    update(model, { type: "chooseDirectory", id: "project-request" })[1],
    [],
  );
  assert.equal(update(model, reply("directory:1"))[0], model);
  [model, effects] = update(model, {
    type: "reply",
    name: "listDirectories",
    sessionId: "directory:2",
    outcome: { kind: "uncertain", reason: "connection lost" },
  });
  assert.equal(model.directoryPicker?.listing?.path, "/home/me");
  assert.equal(model.directoryPicker?.loading, false);
  assert.match(model.directoryPicker?.error ?? "", /connection lost/);
  assert.deepEqual(effects, []);
  [model] = update(model, { type: "closeDirectories" });
  assert.equal(update(model, reply("directory:2"))[0], model);
  [model] = update(model, { type: "openDirectories" });
  assert.equal(update(model, reply("directory:2"))[0], model);
  [model] = update(model, { type: "logout" });
  [model] = update(model, { type: "login", token: "fixture" });
  [model] = update(model, { type: "openDirectories" });
  assert.equal(update(model, reply("directory:3"))[0], model);
});

test("directory picker renders folder navigation, selection and empty states", async () => {
  const { JSDOM } = await import("jsdom");
  const { mountProgram } = await import("@pairshaped/hypertea/testing");
  const { program } = await import("../src/app.ts");
  const dom = new JSDOM('<div id="app"></div>', { url: "http://localhost" });
  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    Node: dom.window.Node,
    Element: dom.window.Element,
    HTMLElement: dom.window.HTMLElement,
    HTMLInputElement: dom.window.HTMLInputElement,
    HTMLTextAreaElement: dom.window.HTMLTextAreaElement,
    HTMLSelectElement: dom.window.HTMLSelectElement,
    requestAnimationFrame: (fn: FrameRequestCallback) => setTimeout(fn, 0),
  });
  const handle = mountProgram({
    node: dom.window.document.getElementById("app")!,
    flags: initial("fixture"),
    program: { ...program, subscriptions: () => [] },
  });
  const button = (text: string) =>
    [...dom.window.document.querySelectorAll<HTMLButtonElement>("button")].find(
      (b) => b.textContent === text,
    )!;
  try {
    await handle.settle();
    const browse = dom.window.document.querySelector<HTMLButtonElement>(
      '[aria-label="Browse folders"]',
    )!;
    assert.ok(browse.querySelector('svg[aria-hidden="true"]'));
    assert.equal(browse.parentElement?.tagName, "HEADER");
    browse.click();
    await handle.settle();
    assert.ok(dom.window.document.querySelector('[role="dialog"]'));
    assert.ok(button("Use this folder").disabled);
    handle.dispatch({
      type: "reply",
      name: "listDirectories",
      sessionId: "directory:1",
      outcome: { kind: "replied", reply: listing },
    });
    await handle.settle();
    assert.ok(button("[..]").disabled);
    button("New folder").click();
    await handle.settle();
    const input = dom.window.document.querySelector<HTMLInputElement>(
      "[data-directory-name]",
    )!;
    assert.ok(input);
    assert.ok(button("Create").disabled);
    assert.deepEqual(handle.effects().at(-1)?.effect, {
      type: "directoryFocus",
      open: true,
      target: "[data-directory-name]",
    });
    const dialog = dom.window.document.querySelector('[role="dialog"]')!;
    const controls = [
      ...dialog.querySelectorAll<HTMLElement>(
        "button:not(:disabled), input:not(:disabled)",
      ),
    ];
    controls.at(-1)!.focus();
    controls.at(-1)!.dispatchEvent(
      new dom.window.KeyboardEvent("keydown", {
        key: "Tab",
        bubbles: true,
        cancelable: true,
      }),
    );
    assert.equal(dom.window.document.activeElement, controls[0]);
    input.dispatchEvent(
      new dom.window.KeyboardEvent("keydown", {
        key: "Escape",
        bubbles: true,
        cancelable: true,
      }),
    );
    await handle.settle();
    assert.ok(dom.window.document.querySelector('[role="dialog"]'));
    assert.equal(
      dom.window.document.querySelector("[data-directory-name]"),
      null,
    );
    assert.deepEqual(handle.effects().at(-1)?.effect, {
      type: "directoryFocus",
      open: true,
      target: "[data-new-directory]",
    });
    button("projects").click();
    await handle.settle();
    assert.deepEqual(handle.effects().at(-1)?.effect, {
      type: "request",
      name: "listDirectories",
      body: { path: "/home/me/projects" },
      sessionId: "directory:3",
    });
    handle.dispatch({
      type: "reply",
      name: "listDirectories",
      sessionId: "directory:3",
      outcome: {
        kind: "replied",
        reply: {
          path: "/home/me/projects",
          parent: "/home/me",
          directories: [],
        },
      },
    });
    await handle.settle();
    assert.ok(!button("[..]").disabled);
    assert.match(dom.window.document.body.textContent ?? "", /No subfolders/);
    button("Use this folder").click();
    await handle.settle();
    assert.equal(dom.window.document.querySelector('[role="dialog"]'), null);
    const sent = handle.effects().at(-1)?.effect;
    assert.ok(sent?.type === "request" && sent.name === "addProject");
    assert.equal((sent.body as { path: string }).path, "/home/me/projects");
    assert.equal(
      dom.window.document.querySelector('.sidebar button[type="submit"]'),
      null,
    );
  } finally {
    handle.stop();
    dom.window.close();
  }
});

test("Home navigation preserves the previous folder when loading fails", () => {
  let [model] = update(initial("fixture"), { type: "openDirectories" });
  [model] = update(model, {
    type: "reply",
    name: "listDirectories",
    sessionId: "directory:1",
    outcome: { kind: "replied", reply: listing },
  });
  [model] = update(model, { type: "openDirectories" });
  assert.equal(model.directoryPicker?.listing, listing);
  [model] = update(model, {
    type: "reply",
    name: "listDirectories",
    sessionId: "directory:2",
    outcome: {
      kind: "rejected",
      error: {
        code: "invalid_request",
        message: "The home folder is unavailable. Check its permissions.",
      },
    },
  });
  assert.equal(model.directoryPicker?.listing, listing);
  assert.match(model.directoryPicker?.error ?? "", /permissions/);
});

test("creation keeps failures and discards pending replies after cancel, close and lock", () => {
  let [model] = update(initial("fixture"), { type: "openDirectories" });
  [model] = update(model, {
    type: "reply",
    name: "listDirectories",
    sessionId: "directory:1",
    outcome: { kind: "replied", reply: listing },
  });
  let effects;
  [model, effects] = update(model, { type: "newDirectory" });
  assert.equal(model.directoryPicker?.name, "");
  assert.deepEqual(effects, [
    { type: "directoryFocus", open: true, target: "[data-directory-name]" },
  ]);
  assert.deepEqual(update(model, { type: "createDirectory" })[1], []);
  [model] = update(model, { type: "directoryName", name: "child" });
  [model] = update(model, { type: "createDirectory" });
  const pending = model;
  const reply = {
    type: "reply",
    name: "createDirectory",
    sessionId: "directory:2",
    outcome: {
      kind: "replied",
      reply: { path: "/home/me/child", parent: "/home/me", directories: [] },
    },
  } as const;
  [model, effects] = update(model, reply);
  assert.equal(model.directoryPicker?.listing?.path, "/home/me/child");
  assert.equal(model.directoryPicker?.name, undefined);
  assert.deepEqual(effects, [
    { type: "directoryFocus", open: true, target: "[data-use-directory]" },
  ]);
  [model] = update(pending, {
    ...reply,
    outcome: {
      kind: "rejected",
      error: {
        code: "invalid_request",
        message: "Folder exists. Choose another name.",
      },
    },
  });
  assert.equal(model.directoryPicker?.name, "child");
  assert.equal(
    model.directoryPicker?.error,
    "Folder exists. Choose another name.",
  );
  for (const type of [
    "cancelDirectory",
    "closeDirectories",
    "logout",
  ] as const) {
    const [canceled] = update(pending, { type });
    assert.equal(update(canceled, reply)[0], canceled);
  }
});
