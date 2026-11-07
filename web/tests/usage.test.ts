import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { mountProgram } from "@pairshaped/hypertea/testing";
import { initial, program, update, type Model } from "../src/app.ts";
import { usageResetSubscription } from "../src/usage.ts";
import type { SessionInfo } from "../src/generated/contract.ts";

const session = (
  agent: SessionInfo["agent"] = "claude",
  usedPercent = 42,
  resetsAt = Math.floor(Date.now() / 1000) + 3600,
): SessionInfo => ({
  sessionId: "s",
  projectId: "p",
  name: "Usage fixture",
  agent,
  workingDirectory: "/fake",
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
    model:
      agent === "claude"
        ? "sonnet"
        : agent === "codex"
          ? "gpt-6.1-sol"
          : "deepseek",
    effort: null,
    usage: {
      context: { usedTokens: 84000, windowTokens: 200000 },
      contextUpdatedAt: 1,
    },
    accountUsage: {
      provider: agent,
      accountId: "account-1",
      weekly: { usedPercent, resetsAt, observedAt: 1 },
    },
  },
});
async function mounted(s: SessionInfo) {
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
    ...initial("fake"),
    active: "s",
    sessions: [s],
    projects: [
      { projectId: "p", name: "Fixture", path: "/fake", createdAt: 1 },
    ],
    models: [
      {
        model: s.status.model,
        label: s.status.model,
        agent: s.agent,
        efforts: [],
      },
    ],
  };
  const handle = mountProgram({
    flags: model,
    node: dom.window.document.querySelector<HTMLElement>("#app")!,
    program,
  });
  await handle.settle();
  return { dom, handle };
}
test("context fills with use while weekly drains to zero, including over-limit observations", async () => {
  const s = session("claude", 125);
  const { dom, handle } = await mounted(s);
  const bars = dom.window.document.querySelectorAll(
    '.toolbar [role="progressbar"]',
  );
  assert.equal(bars.length, 2);
  assert.equal(bars[0]!.getAttribute("aria-valuenow"), "42");
  assert.equal(bars[1]!.getAttribute("aria-valuenow"), "0");
  assert.match(
    dom.window.document.querySelector(".usage-meters")!.textContent!,
    /0%/,
  );
  assert.equal(
    bars[1]!.querySelector("span")!.getAttribute("style"),
    "width: 0%;",
  );
  handle.stop();
  dom.window.close();
});
test("missing and expired data stay unknown rather than appearing as zero", async () => {
  const base = session("codex");
  const s: SessionInfo = {
    ...base,
    status: {
      ...base.status,
      usage: {},
      accountUsage: {
        provider: "codex",
        accountId: "account-1",
        weekly: { usedPercent: 42, resetsAt: 1, observedAt: 1 },
      },
    },
  };
  const { dom, handle } = await mounted(s);
  const bars = dom.window.document.querySelectorAll(
    '.usage-meters [role="progressbar"]',
  );
  assert.equal(bars.length, 2);
  for (const bar of bars) {
    assert.equal(bar.getAttribute("aria-valuenow"), null);
    assert.equal(bar.getAttribute("aria-valuetext"), "Not reported");
  }
  assert.doesNotMatch(
    dom.window.document.querySelector(".usage-meters")!.textContent!,
    /0%/,
  );
  handle.stop();
  dom.window.close();
});
test("a per-chat usage snapshot drives the context meter only, never the allowances", async () => {
  const base = session("codex");
  const s: SessionInfo = {
    ...base,
    status: {
      ...base.status,
      accountUsage: null,
      usage: {
        context: { usedTokens: 84000, windowTokens: 200000 },
        contextUpdatedAt: 1,
      },
    },
  };
  const { dom, handle } = await mounted(s);
  const bars = [
    ...dom.window.document.querySelectorAll('[role="progressbar"]'),
  ];
  assert.equal(bars[0]?.getAttribute("aria-label"), "Context used");
  assert.equal(bars[0]?.getAttribute("aria-valuenow") !== null, true);
  for (const bar of bars.slice(1))
    assert.equal(bar.getAttribute("aria-valuenow"), null);
  assert.equal(dom.window.document.querySelector(".usage-credits"), null);
  handle.stop();
  dom.window.close();
});

test("Pi shows context only, even if a quota is present in malformed cached state", async () => {
  const { dom, handle } = await mounted(session("pi"));
  assert.equal(
    dom.window.document.querySelectorAll('.usage-meters [role="progressbar"]')
      .length,
    1,
  );
  assert.equal(
    dom.window.document.querySelector('[aria-label="Weekly remaining"]'),
    null,
  );
  handle.stop();
  dom.window.close();
});
test("weekly reset subscriptions exist only for a future applicable reset", () => {
  const s = session("claude", 42, 20);
  assert.ok(usageResetSubscription(s, { type: "expired" }, 10000));
  assert.equal(usageResetSubscription(s, { type: "expired" }, 20000), null);
  assert.equal(
    usageResetSubscription({ ...s, agent: "pi" }, { type: "expired" }, 10000),
    null,
  );
});

test("context and weekly fills are red at 90 percent and above only", async () => {
  for (const percent of [89, 89.9, 90, 91, 100]) {
    const base = session("codex", percent);
    const s: SessionInfo = {
      ...base,
      status: {
        ...base.status,
        usage: {
          ...base.status.usage,
          context: {
            usedTokens: Math.round(percent * 1000),
            windowTokens: 100000,
          },
        },
      },
    };
    const { dom, handle } = await mounted(s);
    for (const bar of dom.window.document.querySelectorAll(".usage-track"))
      assert.equal(bar.classList.contains("high"), percent >= 90);
    handle.stop();
    dom.window.close();
  }
});

test("usage labels open styled popovers without hover titles or tab stops", async () => {
  const s = session("codex");
  const { dom, handle } = await mounted(s);
  const labels = [
    ...dom.window.document.querySelectorAll<HTMLButtonElement>(".usage-action"),
  ];
  assert.deepEqual(
    labels.map((b) => b.textContent),
    ["Context", "Weekly"],
  );
  for (const label of labels) {
    assert.equal(label.tabIndex, -1);
    assert.equal(label.closest(".usage-meter")!.hasAttribute("title"), false);
    assert.equal(
      dom.window.document
        .getElementById(label.getAttribute("popovertarget")!)!
        .getAttribute("popover"),
      "auto",
    );
  }
  const context = dom.window.document.querySelector(
    '[aria-label="Compact context"]',
  )!;
  assert.match(context.textContent!, /Compact this chat/);
  assert.doesNotMatch(context.textContent!, /tokens/);
  const weekly = dom.window.document.getElementById("weekly-reset-s")!;
  assert.ok(
    weekly.textContent!.includes(
      new Date(s.status.accountUsage!.weekly!.resetsAt! * 1000).toLocaleString(
        undefined,
        { dateStyle: "full", timeStyle: "short" },
      ),
    ),
  );
  handle.stop();
  dom.window.close();
});

test("confirmed compaction preserves drafts and attachments and uses the delivery path", () => {
  const model: Model = {
    ...initial("fake"),
    active: "s",
    sessions: [session("codex")],
    drafts: { s: "/compact" },
  };
  const [sent, effects] = update(model, {
    type: "compact",
    sessionId: "s",
    id: "compact-id",
  });
  assert.equal(sent.drafts.s, "/compact");
  assert.equal(sent.pending.s!.preserveDraft, true);
  assert.deepEqual(effects[1], {
    type: "request",
    name: "sendMessage",
    sessionId: "s",
    body: { sessionId: "s", messageId: "compact-id", content: "/compact" },
  });
  const [done] = update(sent, {
    type: "reply",
    name: "sendMessage",
    sessionId: "s",
    outcome: { kind: "replied", reply: {} },
  });
  assert.equal(done.drafts.s, "/compact");
  const [failed] = update(sent, {
    type: "reply",
    name: "sendMessage",
    sessionId: "s",
    outcome: { kind: "uncertain" },
  });
  assert.equal(failed.uncertain.s!.preserveDraft, true);
  const [, blocked] = update(failed, {
    type: "compact",
    sessionId: "s",
    id: "duplicate",
  });
  assert.deepEqual(blocked, []);
});

test("five-hour allowance appears only when reported, between context and weekly", async () => {
  const base = session("claude", 37);
  const s: SessionInfo = {
    ...base,
    status: {
      ...base.status,
      accountUsage: {
        ...base.status.accountUsage!,
        fiveHour: { usedPercent: 80, resetsAt: null, observedAt: 1 },
      },
    },
  };
  const { dom, handle } = await mounted(s);
  const bars = [
    ...dom.window.document.querySelectorAll('[role="progressbar"]'),
  ];
  assert.deepEqual(
    bars.map((b) => b.getAttribute("aria-label")),
    ["Context used", "Five-hour remaining", "Weekly remaining"],
  );
  assert.deepEqual(
    bars.map((b) => b.getAttribute("aria-valuenow")),
    ["42", "20", "63"],
  );
  handle.stop();
  dom.window.close();
});

test("credits show any available positive balance regardless of allowances", async () => {
  for (const [usedPercent, balance, reset, visible] of [
    [99, 40000, null, true],
    [100, 40000, null, true],
    [105, 123.45, null, true],
    [100, 0, null, false],
    [100, null, null, false],
    [100, 40000, 1, true],
  ] as const) {
    const base = session("codex");
    const s: SessionInfo = {
      ...base,
      status: {
        ...base.status,
        accountUsage: {
          ...base.status.accountUsage!,
          weekly: { usedPercent, resetsAt: reset, observedAt: 1 },
          credits: { balance, observedAt: 1 },
        },
      },
    };
    const { dom, handle } = await mounted(s);
    const credits = dom.window.document.querySelector(".usage-credits");
    assert.equal(credits !== null, visible);
    if (credits) {
      assert.ok(
        credits.textContent!.includes(Math.floor(balance!).toLocaleString()),
      );
      assert.equal(credits.querySelector('[role="progressbar"]'), null);
    }
    handle.stop();
    dom.window.close();
  }
});

test("the earliest allowance reset expires in the existing subscription slot", () => {
  const base = session("claude", 42, 30);
  const s: SessionInfo = {
    ...base,
    status: {
      ...base.status,
      accountUsage: {
        ...base.status.accountUsage!,
        fiveHour: { usedPercent: 80, resetsAt: 20, observedAt: 1 },
      },
    },
  };
  assert.equal(
    usageResetSubscription(s, { type: "expired" }, 10000)!.key,
    "usage-reset:s:20",
  );
  assert.equal(
    usageResetSubscription(s, { type: "expired" }, 20000)!.key,
    "usage-reset:s:30",
  );
});

test("credits remain visible alongside available, exhausted, and expired allowances", async () => {
  for (const [used, reset, show] of [
    [99, null, true],
    [100, null, true],
    [110, null, true],
    [100, 1, true],
  ] as const) {
    const base = session("codex", 100);
    const s: SessionInfo = {
      ...base,
      status: {
        ...base.status,
        accountUsage: {
          ...base.status.accountUsage!,
          fiveHour: { usedPercent: used, resetsAt: reset, observedAt: 1 },
          credits: { balance: 40000, observedAt: 1 },
        },
      },
    };
    const { dom, handle } = await mounted(s);
    assert.equal(
      dom.window.document.querySelector(".usage-credits") !== null,
      show,
    );
    const bars = dom.window.document.querySelectorAll(".usage-track");
    assert.equal(bars.length, 3);
    assert.equal(bars[2]!.getAttribute("aria-valuenow"), "0");
    assert.equal(
      bars[1]!.getAttribute("aria-valuenow"),
      reset === 1 ? null : String(Math.max(0, 100 - used)),
    );
    handle.stop();
    dom.window.close();
  }
});
