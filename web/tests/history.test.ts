import { test } from "node:test";
import assert from "node:assert/strict";
import { initial, update, subscriptions, type Model } from "../src/app.ts";
import { emptyTimeline, projectEvents } from "../src/timeline.ts";
import {
  limits,
  type StoredEvent,
  type ChatHistoryPage,
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
  model: "sonnet",
  effort: null,
};
const message = (seq: number, text = `message ${seq}`): StoredEvent => ({
  sessionId: "s",
  seq,
  createdAt: 1,
  kind: "agent_event",
  payload: {
    type: "assistant",
    message: { id: `a${seq}`, content: [{ type: "text", text }] },
  },
});
function page(
  model: Model,
  start: number,
  end: number,
  hasMore: boolean,
  cursor = end,
): Model {
  const reply: ChatHistoryPage = {
    sessionId: "s",
    partialId: null,
    status,
    cursor,
    events: Array.from({ length: end - start + 1 }, (_, i) =>
      message(start + i),
    ),
    context: [],
    hasMore,
  };
  return update(model, {
    type: "historyReply",
    sessionId: "s",
    generation: model.history.s!.generation,
    outcome: { kind: "replied", reply },
  })[0];
}
const open = () => update(initial("t"), { type: "select", id: "s" })[0];

test("opening a chat waits for its recent history snapshot before connecting SSE", () => {
  const [model, effects] = update(initial("t"), { type: "select", id: "s" });
  assert.equal(model.history.s?.loading, true);
  assert.equal(subscriptions(model)[1]!.key, "slot:chat-stream");
  assert.equal(effects.at(-1)?.type, "historyRequest");
  const loaded = page(model, 951, 1000, true, 12000);
  assert.equal(loaded.history.s?.timeline.rows.length, 50);
  assert.equal(loaded.cursors.s, 12000);
  assert.equal(subscriptions(loaded)[1]!.key, "session:s");
  assert.equal(
    loaded.events.s?.length,
    0,
    "raw history is not retained alongside projected rows",
  );
});

test("upward and downward paging keep a bounded sorted window without dropping live cursor progress", () => {
  let model = page(open(), 951, 1000, true, 12000);
  for (
    let i = 0;
    i < Math.ceil(limits.historyMaxRows / limits.historyPageSize) + 1;
    i++
  ) {
    const first = model.history.s!.timeline.rows[0]!.seq;
    [model] = update(model, { type: "history", direction: "older" });
    model = page(model, first - 50, first - 1, true, 12000);
  }
  assert.equal(model.history.s!.timeline.rows.length, limits.historyMaxRows);
  assert.equal(model.history.s!.hasNewer, true);
  const last = model.history.s!.timeline.rows.at(-1)!.seq;
  [model] = update(model, { type: "history", direction: "newer" });
  model = page(model, last + 1, last + 50, true, 12500);
  assert.equal(
    model.cursors.s,
    12000,
    "an older-page snapshot must not skip live events",
  );
  assert.equal(model.history.s!.timeline.rows.length, limits.historyMaxRows);
  assert.equal(
    new Set(model.history.s!.timeline.rows.map((r) => r.key)).size,
    limits.historyMaxRows,
  );
  assert.ok(
    model.history.s!.timeline.rows.every(
      (r, i, rows) => i === 0 || rows[i - 1]!.seq < r.seq,
    ),
  );
});

test("live output remains bounded while following and while reading older messages", () => {
  let model = page(open(), 1, 50, false);
  for (let seq = 51; seq <= 1000; seq++)
    [model] = update(model, { type: "event", event: message(seq) });
  assert.equal(model.history.s!.timeline.rows.length, limits.historyMaxRows);
  assert.equal(model.history.s!.hasOlder, true);
  [model] = update(model, { type: "following", value: false });
  const anchor = model.history.s!.timeline.rows[0]!.key;
  for (let seq = 1001; seq <= 1500; seq++)
    [model] = update(model, { type: "event", event: message(seq) });
  assert.equal(
    model.history.s!.timeline.rows[0]!.key,
    anchor,
    "incoming messages cannot evict what the owner is reading",
  );
  assert.equal(model.history.s!.timeline.rows.length, limits.historyMaxRows);
  assert.equal(model.history.s!.hasNewer, true);
  assert.equal(model.cursors.s, 1500);
  const [jumped, effects] = update(model, { type: "bottom" });
  assert.equal(jumped.history.s!.direction, "latest");
  assert.equal(effects[0]?.type, "historyRequest");
});

test("switching and locking ignore stale page replies and release the previous chat cache", () => {
  const first = open();
  const generation = first.history.s!.generation;
  const [other] = update(first, { type: "select", id: "other" });
  assert.equal(other.history.s, undefined);
  assert.equal(
    update(other, {
      type: "historyReply",
      sessionId: "s",
      generation,
      outcome: {
        kind: "replied",
        reply: {
          sessionId: "s",
          partialId: null,
          status,
          cursor: 1,
          events: [message(1)],
          context: [],
          hasMore: false,
        },
      },
    })[0],
    other,
  );
  const [locked] = update(first, { type: "logout" });
  assert.equal(
    update(locked, {
      type: "historyReply",
      sessionId: "s",
      generation,
      outcome: { kind: "uncertain" },
    })[0],
    locked,
  );
});

test("snapshot/live races do not duplicate partial text or regress message revisions", () => {
  const partial: StoredEvent = {
    sessionId: "s",
    seq: 1,
    createdAt: 1,
    kind: "agent_event",
    payload: {
      type: "assistant",
      history_streaming: true,
      message: { id: "a", content: [{ type: "text", text: "hello" }] },
    },
  };
  let timeline = projectEvents(emptyTimeline(), [partial], "Claude", 10);
  timeline = projectEvents(timeline, [
    {
      sessionId: "s",
      seq: 9,
      createdAt: 1,
      kind: "agent_event",
      payload: {
        type: "stream_event",
        event: { type: "content_block_delta", delta: { text: "hello" } },
      },
    },
  ]);
  assert.equal(timeline.rows[0]!.text, "hello");
  timeline = projectEvents(timeline, [
    {
      sessionId: "s",
      seq: 11,
      createdAt: 1,
      kind: "agent_event",
      payload: {
        type: "stream_event",
        event: { type: "content_block_delta", delta: { text: " world" } },
      },
    },
  ]);
  assert.equal(timeline.rows[0]!.text, "hello world");
  const untouched = timeline.rows[0];
  timeline = projectEvents(timeline, [message(12)]);
  assert.equal(
    timeline.rows[0],
    untouched,
    "unrelated rows retain their identity",
  );
});

test("a latest-page reply preserves events and the stream pointer newer than its snapshot", () => {
  let model = page(open(), 1, 50, false, 50);
  [model] = update(model, { type: "history", direction: "latest" });
  [model] = update(model, {
    type: "event",
    event: {
      sessionId: "s",
      seq: 51,
      createdAt: 1,
      kind: "agent_event",
      payload: {
        type: "stream_event",
        event: { type: "message_start", message: { id: "stream" } },
      },
    },
  });
  model = page(model, 1, 50, false, 50);
  assert.equal(model.cursors.s, 51);
  assert.equal(model.history.s!.timeline.partial, "stream");
  [model] = update(model, {
    type: "event",
    event: {
      sessionId: "s",
      seq: 52,
      createdAt: 1,
      kind: "agent_event",
      payload: {
        type: "stream_event",
        event: { type: "content_block_delta", delta: { text: "new response" } },
      },
    },
  });
  assert.equal(model.history.s!.timeline.rows.at(-1)?.text, "new response");
});

test("paging failure keeps visible rows available and can be retried", () => {
  let model = page(open(), 51, 100, true);
  [model] = update(model, { type: "history", direction: "older" });
  [model] = update(model, {
    type: "historyReply",
    sessionId: "s",
    generation: model.history.s!.generation,
    outcome: { kind: "uncertain", reason: "network lost" },
  });
  assert.equal(model.history.s!.timeline.rows.length, 50);
  assert.equal(model.history.s!.loading, false);
  assert.match(model.history.s!.error, /network lost/);
  const [retry, effects] = update(model, {
    type: "history",
    direction: "older",
  });
  assert.equal(retry.history.s!.loading, true);
  assert.equal(effects[0]?.type, "historyRequest");
});

test("sent thumbnails are released with their history window while draft thumbnails survive", () => {
  let model = page(open(), 1, 50, false);
  model = {
    ...model,
    thumbs: { old: "blob:old", draft: "blob:draft" },
    attachments: {
      s: [
        {
          id: "draft",
          name: "draft.png",
          contentType: "image/png",
          size: 1,
          state: "ready",
        },
      ],
    },
  };
  const [switched, effects] = update(model, { type: "select", id: "other" });
  assert.deepEqual(switched.thumbs, { draft: "blob:draft" });
  assert.ok(effects.some((e) => e.type === "discard" && e.url === "blob:old"));
  const [late, discard] = update(switched, {
    type: "thumb",
    id: "old",
    url: "blob:late",
  });
  assert.equal(late, switched);
  assert.deepEqual(discard, [{ type: "discard", id: "old", url: "blob:late" }]);
});

test("returning after a long absence snapshots latest before replaying missed events", () => {
  let model = page(open(), 951, 1000, true, 12000);
  model = { ...model, drafts: { s: "Keep my unfinished reply" } };
  [model] = update(model, { type: "visibility", hidden: true, now: 1000 });
  assert.equal(subscriptions(model)[1]!.key, "slot:chat-stream");
  const [returned, effects] = update(model, {
    type: "visibility",
    hidden: false,
    now: 3_601_000,
  });
  assert.equal(subscriptions(returned)[1]!.key, "slot:chat-stream");
  assert.equal(returned.drafts.s, "Keep my unfinished reply");
  assert.ok(
    effects.some(
      (e) => e.type === "historyRequest" && e.direction === "latest",
    ),
  );
  const resumed = page(returned, 4951, 5000, true, 60000);
  assert.equal(resumed.cursors.s, 60000);
  assert.equal(resumed.history.s!.timeline.rows.length, 50);
  assert.equal(resumed.history.s!.timeline.rows[0]!.seq, 4951);
  assert.equal(subscriptions(resumed)[1]!.key, "session:s");
});

test("brief app switches keep the reading window and resume from its cursor", () => {
  let model = page(open(), 951, 1000, true, 12000);
  [model] = update(model, { type: "visibility", hidden: true, now: 1000 });
  const [returned, effects] = update(model, {
    type: "visibility",
    hidden: false,
    now: 2000,
  });
  assert.equal(returned.history.s!.timeline.rows[0]!.seq, 951);
  assert.equal(returned.cursors.s, 12000);
  assert.equal(subscriptions(returned)[1]!.key, "session:s");
  assert.ok(!effects.some((e) => e.type === "historyRequest"));
});

test("resume cutoff is fifteen minutes and includes the exact boundary", () => {
  assert.equal(limits.historyResumeLatestMs, 15 * 60 * 1000);
  for (const away of [
    limits.historyResumeLatestMs - 1,
    limits.historyResumeLatestMs,
  ]) {
    const loaded = page(open(), 951, 1000, true, 12000);
    const hidden = update(loaded, {
      type: "visibility",
      hidden: true,
      now: 1000,
    })[0];
    const [returned, effects] = update(hidden, {
      type: "visibility",
      hidden: false,
      now: 1000 + away,
    });
    assert.equal(
      effects.some((e) => e.type === "historyRequest"),
      away >= limits.historyResumeLatestMs,
    );
    assert.equal(returned.drafts, loaded.drafts);
  }
});

test("long-absence snapshot supersedes pending paging and ignores late hidden events", () => {
  let model = page(open(), 951, 1000, true, 12000);
  [model] = update(model, { type: "history", direction: "older" });
  const oldGeneration = model.history.s!.generation;
  [model] = update(model, { type: "visibility", hidden: true, now: 0 });
  const ignored = update(model, { type: "event", event: message(12001) })[0];
  assert.equal(ignored, model);
  [model] = update(model, {
    type: "visibility",
    hidden: false,
    now: 3_600_000,
  });
  assert.ok(model.history.s!.generation > oldGeneration);
  assert.equal(model.history.s!.direction, "latest");
  assert.equal(
    update(model, { type: "event", event: message(12002) })[0],
    model,
  );
  const stale = update(model, {
    type: "historyReply",
    sessionId: "s",
    generation: oldGeneration,
    outcome: { kind: "uncertain", reason: "old request" },
  })[0];
  assert.equal(stale, model);
});

test("idle reset clears rendered rows and cursors immediately through the selection path", () => {
  const loaded = {
    ...page(open(), 951, 1000, true, 12000),
    drafts: { s: "unfinished" },
  };
  const [reset, effects] = update(loaded, {
    type: "visibility",
    hidden: false,
    now: limits.historyResumeLatestMs,
    idle: true,
  });
  assert.equal(reset.history.s!.timeline.rows.length, 0);
  assert.deepEqual(reset.cursors, {});
  assert.deepEqual(reset.events, {});
  assert.equal(reset.drafts.s, "unfinished");
  assert.equal(subscriptions(reset)[1]!.key, "slot:chat-stream");
  assert.ok(
    effects.some(
      (e) => e.type === "historyRequest" && e.direction === "latest",
    ),
  );
  const latest = page(reset, 4951, 5000, true, 60000);
  assert.ok(
    effects.some((e) => e.type === "composer" && e.text === "unfinished"),
  );
  assert.equal(latest.history.s!.following, true);
});

test("hiding a chat with weekly usage preserves every subscription slot", () => {
  const loaded = page(open(), 1, 50, false);
  const visible: Model = {
    ...loaded,
    sessions: [
      {
        sessionId: "s",
        projectId: "p",
        name: "Usage chat",
        agent: "codex",
        workingDirectory: "/fake",
        agentSessionId: null,
        createdAt: 1,
        status: {
          ...status,
          accountUsage: {
            provider: "codex",
            accountId: "account-1",
            weekly: {
              usedPercent: 42,
              resetsAt: Math.floor(Date.now() / 1000) + 3600,
              observedAt: 1,
            },
          },
        },
      },
    ],
  };
  const hidden = update(visible, {
    type: "visibility",
    hidden: true,
    now: Date.now(),
  })[0];
  assert.equal(
    subscriptions(hidden).length,
    subscriptions(visible).length,
    "a conditional usage timer must not shift the following browser subscriptions on hide",
  );
});
