import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { mountProgram } from "@pairshaped/hypertea/testing";
import { initial, program, update, type Model } from "../src/app.ts";
import type { SessionInfo } from "../src/generated/contract.ts";

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
const shell = (dom: JSDOM) => dom.window.document.querySelector(".shell")!;

test("with a chat open the phone drawer is closed; the Chats button opens it", async () => {
  const { dom, handle } = await mounted({ ...base, active: "s1" });
  assert.equal(shell(dom).classList.contains("sidebar-open"), false);
  const button = dom.window.document.querySelector<HTMLElement>(
    "button.chats-toggle",
  );
  assert.ok(button);
  assert.equal(button.getAttribute("aria-expanded"), "false");
  button.dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true }));
  await handle.settle();
  assert.equal(handle.model().sidebarOpen, true);
  assert.equal(shell(dom).classList.contains("sidebar-open"), true);
  handle.stop();
  dom.window.close();
});

test("the drawer shows by itself when there is no chat to look at, or no login yet", async () => {
  const none = await mounted(base);
  assert.equal(shell(none.dom).classList.contains("sidebar-open"), true);
  none.handle.stop();
  none.dom.window.close();
  const locked = await mounted({ ...base, token: "", active: "s1" });
  assert.ok(
    locked.dom.window.document.querySelector(
      '[role="dialog"][aria-modal="true"]',
    ),
  );
  assert.equal(locked.dom.window.document.querySelector(".shell"), null);
  locked.handle.stop();
  locked.dom.window.close();
});

test("picking a chat closes the drawer", () => {
  const [open] = update({ ...base, active: "s1" }, { type: "toggleSidebar" });
  assert.equal(open.sidebarOpen, true);
  const [picked] = update(open, { type: "select", id: "s2" });
  assert.equal(picked.active, "s2");
  assert.equal(picked.sidebarOpen, false);
});

test("the in-drawer close button exists only when a chat is open to go back to", async () => {
  const withChat = await mounted({ ...base, active: "s1", sidebarOpen: true });
  assert.ok(
    withChat.dom.window.document.querySelector(".sidebar button.sidebar-close"),
  );
  withChat.dom.window.document
    .querySelector<HTMLElement>(".sidebar button.sidebar-close")!
    .dispatchEvent(
      new withChat.dom.window.MouseEvent("click", { bubbles: true }),
    );
  await withChat.handle.settle();
  assert.equal(withChat.handle.model().sidebarOpen, false);
  withChat.handle.stop();
  withChat.dom.window.close();
  const without = await mounted(base);
  assert.equal(
    without.dom.window.document.querySelector(".sidebar-close"),
    null,
  );
  without.handle.stop();
  without.dom.window.close();
});

for (const active of ["s1", null]) {
  test(`Browse folders reveals the picker and closes the drawer with active chat ${active}`, async () => {
    const { dom, handle } = await mounted({
      ...base,
      active,
      sidebarOpen: true,
    });
    dom.window.document
      .querySelector<HTMLElement>("button.browse-folders")!
      .click();
    await handle.settle();
    assert.ok(
      dom.window.document.querySelector(
        '[role="dialog"][aria-label="Choose project folder"]',
      ),
    );
    assert.equal(handle.model().sidebarOpen, false);
    assert.equal(shell(dom).classList.contains("sidebar-open"), false);
    handle.dispatch({ type: "closeDirectories" });
    await handle.settle();
    assert.equal(
      shell(dom).classList.contains("sidebar-open"),
      active === null,
    );
    handle.stop();
    dom.window.close();
  });
}

for (const pushState of ["available", "enabled"] as const) {
  test(`the ${pushState} notification toggle uses a bell icon and keeps its accessible action`, async () => {
    const { dom, handle } = await mounted({ ...base, active: "s1", pushState });
    const button = dom.window.document.querySelector<HTMLButtonElement>(
      ".notification-toggle",
    )!;
    assert.ok(button.querySelector('svg[aria-hidden="true"]'));
    assert.equal(button.textContent, "");
    const close = dom.window.document.querySelector(".sidebar-close");
    assert.equal(
      dom.window.document.querySelector(".sidebar > header")?.firstElementChild,
      close,
    );
    assert.equal(close?.nextElementSibling, button);
    assert.equal(
      button.getAttribute("aria-label"),
      pushState === "enabled"
        ? "Disable notifications"
        : "Enable notifications",
    );
    assert.equal(
      button.getAttribute("aria-pressed"),
      String(pushState === "enabled"),
    );
    button.click();
    await handle.settle();
    assert.ok(
      handle
        .effects()
        .some(
          ({ effect }) =>
            effect.type === "togglePush" &&
            effect.enabled === (pushState === "enabled"),
        ),
    );
    handle.stop();
    dom.window.close();
  });
}
