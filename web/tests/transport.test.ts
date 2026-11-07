import { test } from "node:test";
import assert from "node:assert/strict";
import { chatStream } from "../src/stream.ts";
import { initial, update, subscriptions } from "../src/app.ts";
import { postCommand } from "../src/http.ts";
import { parseStoredEvent } from "../src/generated/contract.ts";
const context = { call: async () => null };
function response(text: string): Response {
  return new Response(text, {
    headers: { "content-type": "text/event-stream" },
  });
}
const pause = () => new Promise((resolve) => setTimeout(resolve, 5));
test("reconnect resumes from the last validated event; key excludes cursor; leaving aborts reads", async () => {
  const cursors: string[] = [];
  const bearers: string[] = [];
  const messages: number[] = [];
  let calls = 0;
  let aborted = false;
  const fake: typeof fetch = async (_url, init) => {
    cursors.push(new Headers(init?.headers).get("Last-Event-ID")!);
    bearers.push(new Headers(init?.headers).get("Authorization")!);
    calls++;
    if (calls === 1) return response("id: 1\ndata: 1\n\nid: 2\ndata: partial");
    if (calls === 2) return response("id: 2\ndata: 2\n\nid: 3\ndata: 3\n\n");
    return new Promise((_resolve, reject) =>
      init!.signal!.addEventListener(
        "abort",
        () => {
          aborted = true;
          reject(new Error("aborted"));
        },
        { once: true },
      ),
    );
  };
  const options = {
    key: "session:s",
    endpoint: "/sessions/s/events",
    token: "t",
    after: "0",
    decode: (record: { id: string; data: string }) => ({
      type: "event",
      seq: Number(record.data),
    }),
    health: () => ({ type: "health", seq: -1 }),
    fetch: fake,
    retryMs: 1,
  };
  const subscription = chatStream(options);
  assert.equal(subscription.key, chatStream({ ...options, after: "99" }).key);
  const dispose = subscription.subscribe((msg) => {
    if (msg.type === "event") messages.push(msg.seq);
  }, context);
  for (let i = 0; i < 100 && calls < 3; i++) await pause();
  dispose();
  await pause();
  assert.deepEqual(messages, [1, 2, 3]);
  assert.deepEqual(bearers, ["Bearer t", "Bearer t", "Bearer t"]);
  assert.deepEqual(cursors, ["0", "1", "3"]);
  assert.ok(aborted);
});
test("malformed event does not advance the reconnect cursor", async () => {
  let calls = 0;
  const cursors: string[] = [];
  const fake: typeof fetch = async (_url, init) => {
    cursors.push(new Headers(init?.headers).get("Last-Event-ID")!);
    calls++;
    return response("id: 8\ndata: {}\n\n");
  };
  const sub = chatStream({
    key: "s",
    endpoint: "/events",
    token: "t",
    after: "7",
    decode: (r) => ({
      type: "event",
      value: parseStoredEvent(JSON.parse(r.data)),
    }),
    health: () => ({ type: "health" }),
    onCursor: () => assert.fail("invalid event advanced cursor"),
    fetch: fake,
    retryMs: 1,
  });
  const dispose = sub.subscribe((msg) => {
    if (msg.type === "event") assert.fail("invalid event dispatched");
  }, context);
  for (let i = 0; i < 100 && calls < 2; i++) await pause();
  dispose();
  assert.ok(calls >= 2);
  assert.ok(cursors.every((c) => c === "7"));
});
test("HTTP network failure stays uncertain; explicit retry preserves UUID and never replays itself", async () => {
  const bodies: unknown[] = [];
  const body = {
    sessionId: "s",
    messageId: "00000000-0000-4000-8000-000000000001",
    content: "hello",
  };
  const fake: typeof fetch = async (_url, init) => {
    bodies.push(JSON.parse(String(init?.body)));
    throw new Error("reply lost");
  };
  assert.deepEqual(
    await postCommand("/commands/sendMessage", body, "t", (v) => v, fake),
    {
      kind: "uncertain",
      reason:
        "could not reach the server (reply lost); check the connection and the Tailscale link",
    },
  );
  assert.equal(bodies.length, 1);
  await postCommand("/commands/sendMessage", body, "t", (v) => v, fake);
  assert.deepEqual(bodies, [body, body]);
});
test("HTTP typed errors and malformed replies distinguish rejection from uncertainty", async () => {
  assert.deepEqual(
    await postCommand(
      "/command",
      {},
      "t",
      (v) => v,
      async () =>
        new Response('{"code":"invalid","message":"bad"}', { status: 400 }),
    ),
    { kind: "rejected", error: { code: "invalid", message: "bad" } },
  );
  assert.deepEqual(
    await postCommand(
      "/command",
      {},
      "t",
      (v) => v,
      async () => new Response("broken"),
    ),
    {
      kind: "uncertain",
      reason:
        "the server answered HTTP 200 with something that is not a BobLM reply (a proxy or gateway in front of it may have answered instead)",
    },
  );
});
test("a stream the server closes is reported, then resumed from the last event", async () => {
  const cursors: string[] = [];
  const problems: string[] = [];
  let calls = 0;
  const fake: typeof fetch = async (_url, init) => {
    cursors.push(new Headers(init?.headers).get("Last-Event-ID")!);
    calls++;
    if (calls === 1) return response("id: 5\ndata: 5\n\n");
    return new Promise(() => {});
  };
  const sub = chatStream<{ type: string; seq?: number; text?: string | null }>({
    key: "closed",
    endpoint: "/events",
    token: "t",
    after: "0",
    decode: (record) => ({ type: "event", seq: Number(record.data) }),
    health: (health) => ({
      type: "health",
      text: health.kind === "problem" ? health.text : null,
    }),
    fetch: fake,
    retryMs: 1,
  });
  const dispose = sub.subscribe((msg) => {
    if (msg.type === "health" && msg.text) problems.push(msg.text);
  }, context);
  for (let i = 0; i < 100 && calls < 2; i++) await pause();
  dispose();
  assert.deepEqual(cursors, ["0", "5"]);
  assert.equal(problems.length, 1);
  assert.match(problems[0]!, /ended/);
});
test("the streams sit in fixed slots, keyed by resource and never by cursor", () => {
  const model = { ...initial("t"), listCursor: 4, listGeneration: 1 };
  const locked = subscriptions({ ...model, token: "" });
  assert.deepEqual(
    locked.map((s) => s.key),
    [
      "slot:list",
      "slot:chat-stream",
      "slot:usage-reset",
      "slot:subagent-stream",
    ],
    "a locked page still holds all four slots",
  );
  const idle = subscriptions(model);
  assert.equal(idle.length, 4);
  assert.equal(
    idle[1]?.key,
    "slot:chat-stream",
    "no open chat, an empty slot in its place",
  );
  const open = subscriptions({ ...model, active: "s", cursors: { s: 9 } });
  const later = subscriptions({
    ...model,
    listCursor: 40,
    active: "s",
    cursors: { s: 90 },
  });
  const keys = (list: typeof open) => list.map((s) => s.key);
  assert.deepEqual(keys(open), [
    "list:/sessions/events:1",
    "session:s",
    "slot:usage-reset",
    "slot:subagent-stream",
  ]);
  assert.deepEqual(keys(open), keys(later));
});

test("the list stream waits for a fresh snapshot even with a cached sidebar", () => {
  const model = initial("t");
  assert.equal(subscriptions(model)[0]?.key, "slot:list");
  const [loaded] = update(model, {
    type: "reply",
    name: "listSessions",
    sessionId: "",
    outcome: {
      kind: "replied",
      reply: { sessions: [], projects: [], models: [], cursor: 5000 },
    } as never,
  });
  assert.equal(loaded.listCursor, 5000);
  assert.equal(subscriptions(loaded)[0]?.key, "list:/sessions/events:1");
  const [refreshed] = update(loaded, {
    type: "reply",
    name: "listSessions",
    sessionId: "",
    outcome: {
      kind: "replied",
      reply: { sessions: [], projects: [], models: [], cursor: 6000 },
    } as never,
  });
  assert.equal(refreshed.listCursor, 6000);
  assert.equal(subscriptions(refreshed)[0]?.key, "list:/sessions/events:2");
});

test("returning to a hidden page gets a fresh list before resuming its stream", () => {
  const model = { ...initial("t"), listGeneration: 3, listCursor: 4000 };
  const [hidden] = update(model, {
    type: "visibility",
    hidden: true,
    now: 1000,
    idle: false,
  });
  assert.equal(subscriptions(hidden)[0]?.key, "slot:list");
  const [visible, effects] = update(hidden, {
    type: "visibility",
    hidden: false,
    now: 2000,
    idle: false,
  });
  assert.equal(subscriptions(visible)[0]?.key, "slot:list");
  assert.ok(
    effects.some((e) => e.type === "request" && e.name === "listSessions"),
  );
});
