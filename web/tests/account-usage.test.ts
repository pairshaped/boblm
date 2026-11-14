import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { mountProgram } from "@pairshaped/hypertea/testing";
import { initial, program, update, type Model } from "../src/app.ts";
import type { AccountUsageInfo } from "../src/generated/contract.ts";

const now = Math.floor(Date.now() / 1000);
const accounts: AccountUsageInfo[] = [
  {
    label: "Codex plan",
    kind: "plan",
    provider: "codex",
    fiveHour: {
      usedPercent: 25,
      resetsAt: now + 3 * 3600,
      observedAt: Date.now() - 180_000,
    },
    weekly: {
      usedPercent: 40,
      resetsAt: now + 2 * 86400 + 4 * 3600 + 60,
      observedAt: Date.now() - 180_000,
    },
    credits: { balance: 1200, observedAt: Date.now() },
  },
  {
    label: "Claude plan A",
    kind: "plan",
    provider: "claude",
    fiveHour: null,
    weekly: null,
    credits: null,
  },
  {
    label: "OpenAI API",
    kind: "api",
    provider: "codex",
    fiveHour: null,
    weekly: null,
    credits: null,
  },
];

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
const reply = (generation: number, body: unknown) =>
  ({
    type: "reply",
    name: "accountUsage",
    sessionId: "",
    orderGeneration: generation,
    outcome: { kind: "replied", reply: body },
  }) as const;

test("the clock button replaces Lock, opens the modal, and Escape and Close dismiss it", async () => {
  const { dom, handle } = await mounted(initial("t"));
  const document = dom.window.document;
  try {
    assert.equal(
      document.querySelector('[aria-label="Lock"].header-icon'),
      null,
    );
    const clock = document.querySelector<HTMLButtonElement>(
      "#open-account-usage",
    )!;
    assert.equal(clock.getAttribute("aria-label"), "Account usage");
    assert.equal(clock.getAttribute("aria-haspopup"), "dialog");
    clock.click();
    await handle.settle();
    const dialog = document.querySelector("#account-usage-dialog")!;
    assert.equal(dialog.getAttribute("role"), "dialog");
    assert.equal(dialog.getAttribute("aria-modal"), "true");
    assert.ok(
      handle
        .effects()
        .some(
          (e) =>
            e.effect.type === "request" && e.effect.name === "accountUsage",
        ),
    );
    dialog.dispatchEvent(
      new dom.window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
    );
    await handle.settle();
    assert.equal(document.querySelector("#account-usage-dialog"), null);
    assert.ok(
      handle
        .effects()
        .some(
          (e) =>
            e.effect.type === "focusAccountUsage" && e.effect.open === false,
        ),
    );
    clock.click();
    await handle.settle();
    document
      .querySelector<HTMLButtonElement>(
        'button[aria-label="Close account usage"]',
      )!
      .click();
    await handle.settle();
    assert.equal(document.querySelector("#account-usage-dialog"), null);
  } finally {
    handle.stop();
    dom.window.close();
  }
});

test("the modal shows remaining percentages, reset text, API and no-reading lines", async () => {
  const { dom, handle } = await mounted(initial("t"));
  const document = dom.window.document;
  try {
    document.querySelector<HTMLButtonElement>("#open-account-usage")!.click();
    await handle.settle();
    assert.match(
      document.querySelector("#account-usage-dialog")!.textContent!,
      /Loading usage/,
    );
    handle.dispatch(reply(1, { accounts }));
    await handle.settle();
    const text = document.querySelector("#account-usage-dialog")!.textContent!;
    assert.match(text, /Codex plan/);
    assert.match(text, /75%/);
    assert.match(text, /60%/);
    assert.match(text, /Resets in 2d 4h, /);
    assert.match(text, /as of 3m ago/);
    assert.ok(!/\.,/.test(text));
    assert.match(text, /Credits1,200/);
    assert.match(text, /No readings yet\./);
    assert.match(text, /Allowance is not reported for API accounts\./);
    assert.ok(!/Loading usage/.test(text));
  } finally {
    handle.stop();
    dom.window.close();
  }
});

test("a failed request shows the error and Try again requests again", () => {
  const [open] = update(initial("t"), { type: "openAccountUsage" });
  const [failed] = update(open, {
    type: "reply",
    name: "accountUsage",
    sessionId: "",
    orderGeneration: open.accountUsageGeneration,
    outcome: { kind: "uncertain" },
  });
  assert.match(failed.accountUsageError, /could not be loaded/);
  assert.equal(failed.accountUsageLoading, false);
  const [again, effects] = update(failed, { type: "reloadAccountUsage" });
  assert.equal(again.accountUsageError, "");
  assert.equal(again.accountUsageLoading, true);
  assert.ok(
    effects.some((e) => e.type === "request" && e.name === "accountUsage"),
  );
});

test("stale replies are discarded after a reopen or close", () => {
  const [first] = update(initial("t"), { type: "openAccountUsage" });
  const stale = reply(first.accountUsageGeneration, { accounts });
  const [closed] = update(first, { type: "closeAccountUsage" });
  assert.deepEqual(update(closed, stale)[0].accountUsage, []);
  const [second] = update(closed, { type: "openAccountUsage" });
  assert.deepEqual(update(second, stale)[0].accountUsage, []);
  const [fresh] = update(
    second,
    reply(second.accountUsageGeneration, { accounts }),
  );
  assert.equal(fresh.accountUsage.length, 3);
});

test("an expired window is unavailable, not a fresh allowance", async () => {
  const model: Model = {
    ...initial("t"),
    accountUsageOpen: true,
    accountUsage: [
      {
        ...accounts[0]!,
        fiveHour: { usedPercent: 90, resetsAt: now - 10, observedAt: 1 },
        weekly: null,
        credits: null,
      },
    ],
  };
  const { dom, handle } = await mounted(model);
  try {
    const text = dom.window.document.querySelector(
      "#account-usage-dialog",
    )!.textContent!;
    assert.match(text, /has reset, waiting for a new reading/);
    assert.ok(!/100%/.test(text));
  } finally {
    handle.stop();
    dom.window.close();
  }
});

test("the backdrop clears the sidebar and the modal has no Lock button", async () => {
  const model: Model = {
    ...initial("t"),
    accountUsageOpen: true,
    accountUsage: [],
  };
  const { dom, handle } = await mounted(model);
  try {
    const dialog = dom.window.document.querySelector("#account-usage-dialog")!;
    const scroller = dialog.querySelector(".account-usage")!;
    assert.ok(scroller);
    assert.equal(
      [...dialog.querySelectorAll("button")].find(
        (b) => b.textContent === "Lock",
      ),
      undefined,
    );
    assert.equal(dialog.querySelector("footer"), null);
    assert.ok(
      dialog.parentElement!.classList.contains("account-usage-backdrop"),
    );
  } finally {
    handle.stop();
    dom.window.close();
  }
});

test("credits keep up to two fraction digits", async () => {
  const model: Model = {
    ...initial("t"),
    accountUsageOpen: true,
    accountUsage: [
      { ...accounts[0]!, credits: { balance: 12.5, observedAt: 1 } },
    ],
  };
  const { dom, handle } = await mounted(model);
  try {
    assert.match(
      dom.window.document.querySelector("#account-usage-dialog")!.textContent!,
      /Credits12\.5/,
    );
  } finally {
    handle.stop();
    dom.window.close();
  }
});
