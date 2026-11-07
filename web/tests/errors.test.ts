import { test } from "node:test";
import assert from "node:assert/strict";
import * as contract from "../src/generated/contract.ts";
import { postCommand, putUpload } from "../src/http.ts";
import { initial, update } from "../src/app.ts";

const body = {
  sessionId: "s",
  messageId: "00000000-0000-4000-8000-000000000001",
  content: "hello",
};
const message = (fn: () => unknown) => {
  try {
    fn();
  } catch (error) {
    return (error as Error).message;
  }
  throw new Error("expected a failure");
};

test("a reply that does not decode names the command and the field that is wrong", () => {
  const missing = message(() =>
    contract.parseSendMessage({ sessionId: "s", content: "x" }),
  );
  assert.match(missing, /SendMessage/);
  assert.match(missing, /messageId/);
  assert.match(missing, /missing/i);
  const wrongType = message(() =>
    contract.parseSendMessage({ ...body, content: 5 }),
  );
  assert.match(wrongType, /content/);
  assert.match(wrongType, /string/);
  const extra = message(() =>
    contract.parseSendMessage({ ...body, version: 2 }),
  );
  assert.match(extra, /version/);
  assert.match(extra, /not allowed|unexpected/i);
  const tooLong = message(() =>
    contract.parseSendMessage({ ...body, content: "x".repeat(65537) }),
  );
  assert.match(tooLong, /content/);
  assert.match(tooLong, /65536/);
  const badEnum = message(() =>
    contract.parseListSessionsResponse({
      models: [{ model: "x", label: "x", agent: "gemini", efforts: [] }],
      projects: [],
      sessions: [],
    }),
  );
  assert.match(badEnum, /agent/);
  assert.match(badEnum, /claude|codex/);
});

const reason = (outcome: { kind: string; reason?: string }) => {
  assert.equal(outcome.kind, "uncertain");
  return outcome.reason ?? "";
};

test("transport failures say what happened instead of a bare uncertain", async () => {
  const unreachable = (async () => {
    throw new TypeError("fetch failed");
  }) as unknown as typeof fetch;
  assert.match(
    reason(await postCommand("/commands/x", {}, "t", (v) => v, unreachable)),
    /could not reach the server/i,
  );
  const html = (async () =>
    new Response("<html>Bad gateway</html>", {
      status: 502,
    })) as unknown as typeof fetch;
  const proxied = reason(
    await postCommand("/commands/x", {}, "t", (v) => v, html),
  );
  assert.match(proxied, /502/);
  assert.match(proxied, /proxy|not a boblm reply/i);
  const wrongShape = (async () =>
    new Response(JSON.stringify({ unexpected: true }), {
      status: 200,
    })) as unknown as typeof fetch;
  const shaped = reason(
    await postCommand(
      "/commands/sendMessage",
      {},
      "t",
      contract.parseSendMessageResponse,
      wrongShape,
    ),
  );
  assert.match(shaped, /SendMessageResponse/);
  assert.match(shaped, /messageId|accepted|status/);
  const tooBig = (async () =>
    new Response("x".repeat(1_048_577), {
      status: 200,
    })) as unknown as typeof fetch;
  assert.match(
    reason(await postCommand("/commands/x", {}, "t", (v) => v, tooBig)),
    /larger than|too large|1 MiB/i,
  );
  const upload = reason(
    await putUpload(
      "s",
      "10000000-0000-4000-8000-000000000001",
      new File(["x"], "a.txt"),
      "t",
      (v) => v,
      unreachable,
    ),
  );
  assert.match(upload, /could not reach the server/i);
});

test("a send with no usable reply tells the owner why and what Retry will do", () => {
  const model = { ...initial("t"), active: "s", drafts: { s: "hello" } };
  const [sending] = update(model, { type: "send", id: "first" });
  const [failed] = update(sending, {
    type: "reply",
    name: "sendMessage",
    sessionId: "s",
    outcome: {
      kind: "uncertain",
      reason:
        "the server answered HTTP 502 with something that is not a boblm reply",
    },
  });
  const text = failed.sendError.s ?? "";
  assert.match(text, /HTTP 502/);
  assert.match(text, /whether it received your message/i);
  assert.match(text, /same ID/i);
  assert.doesNotMatch(text, /Outcome uncertain\./);
  // Other commands explain too.
  const [other] = update(
    { ...model, pending: {} },
    {
      type: "reply",
      name: "archiveSession",
      sessionId: "s",
      outcome: {
        kind: "uncertain",
        reason: "could not reach the server (fetch failed)",
      },
    },
  );
  assert.match(other.error, /archiveSession|archive/i);
  assert.match(other.error, /could not reach the server/);
  assert.match(other.error, /may or may not/i);
});

import { chatStream, describeStreamError } from "../src/stream.ts";

/** A healthy stream: it delivers `text`, then stays open the way a live server's does. */
const openStream = (text: string) =>
  new Response(
    new ReadableStream({
      start(controller) {
        controller.enqueue(new TextEncoder().encode(text));
      },
    }),
    { headers: { "content-type": "text/event-stream" } },
  );
const stream = (init: ResponseInit, body = "") =>
  new Response(body, {
    headers: { "content-type": "text/event-stream" },
    ...init,
  });
type Seen = { type: string; text?: string | null };
/** Open a stream against a canned response and return what it reported first. */
const firstProblem = async (answer: () => Response | Promise<Response>) => {
  const seen: string[] = [];
  const subscription = chatStream<Seen>({
    key: "problem",
    endpoint: "/sessions/events",
    token: "t",
    after: "0",
    decode: () => ({ type: "event" }),
    health: (health) => ({
      type: "connection",
      text: health.kind === "problem" ? health.text : null,
    }),
    fetch: (async () => answer()) as unknown as typeof fetch,
    retryMs: 1000,
  });
  const stop = subscription.subscribe(
    (msg) => msg.text && seen.push(msg.text),
    {} as never,
  );
  for (let i = 0; i < 100 && seen.length === 0; i++)
    await new Promise((resolve) => setTimeout(resolve, 5));
  stop();
  return seen[0] ?? "";
};

test("live-stream failures say what the server or network did, in boblm's words", async () => {
  const denied = await firstProblem(() => stream({ status: 401 }));
  assert.match(denied, /HTTP 401/);
  assert.match(denied, /token was rejected/);
  assert.match(denied, /boblm init/);
  const gone = await firstProblem(() => stream({ status: 502 }));
  assert.equal(
    gone,
    "the server answered HTTP 502 when opening the live stream",
  );
  const wrongType = await firstProblem(
    () =>
      new Response("<html></html>", {
        status: 200,
        headers: { "content-type": "text/html" },
      }),
  );
  assert.match(
    wrongType,
    /answered with text\/html, which is not an event stream/,
  );
  assert.match(wrongType, /proxy/);
  const huge = await firstProblem(() =>
    stream({}, `data: ${"x".repeat(1_048_700)}\n\n`),
  );
  assert.match(huge, /larger than the 1 MiB limit/);
  const unreachable = await firstProblem(() => {
    throw new TypeError("fetch failed");
  });
  assert.match(unreachable, /could not reach the server \(fetch failed\)/);
  assert.match(unreachable, /Tailscale/);
  const empty = await firstProblem(
    () =>
      new Response(null, { headers: { "content-type": "text/event-stream" } }),
  );
  assert.match(empty, /sent no stream/);
  const garbled = await firstProblem(
    () =>
      new Response(new Uint8Array([0xff, 0xfe, 0x0a, 0x0a]), {
        headers: { "content-type": "text/event-stream" },
      }),
  );
  assert.match(garbled, /not valid UTF-8/);
  const dropped = await firstProblem(
    () =>
      new Response(
        new ReadableStream({
          start(controller) {
            controller.enqueue(new TextEncoder().encode("id: 1\ndata: x\n\n"));
            controller.error(new Error("socket closed"));
          },
        }),
        { headers: { "content-type": "text/event-stream" } },
      ),
  );
  assert.match(dropped, /dropped while the live stream was open/);
});

test("errors that did not come from the transport keep their own text", () => {
  assert.equal(describeStreamError(new Error("boom"), "boom"), "boom");
  assert.equal(describeStreamError("odd", "odd"), "odd");
});

test("the app reports a broken live stream once, with its cause, and clears it on recovery", async () => {
  let calls = 0;
  const fakeFetch = (async () => {
    calls += 1;
    if (calls <= 2) throw new TypeError("fetch failed");
    return openStream("id: 1\nevent: x\ndata: {}\n\n");
  }) as unknown as typeof fetch;
  const subscription = chatStream<Seen>({
    key: "t",
    endpoint: "/sessions/events",
    token: "t",
    after: "0",
    decode: () => ({ type: "event" }),
    health: (health) => ({
      type: "connection",
      text: health.kind === "problem" ? health.text : null,
    }),
    fetch: fakeFetch,
    retryMs: 1,
  });
  const seen: Seen[] = [];
  const stop = subscription.subscribe((msg) => seen.push(msg), {} as never);
  await new Promise((resolve) => setTimeout(resolve, 80));
  stop();
  const connection = seen.filter((m) => m.type === "connection");
  assert.equal(connection.length >= 2, true);
  assert.match(String(connection[0]?.text), /could not reach the server/i);
  assert.equal(
    connection.filter((m) => m.text).length,
    1,
    "the same problem is reported once",
  );
  assert.equal(connection.at(-1)?.text, null, "recovery clears it");
});

test("a broken live stream is shown in the chat and cleared when it recovers", () => {
  const model = { ...initial("t"), active: "s" };
  const [broken] = update(model, {
    type: "connection",
    stream: "this chat",
    health: {
      kind: "problem",
      text: "could not reach the server (fetch failed)",
    },
  });
  assert.deepEqual(broken.connection, {
    "this chat": "could not reach the server (fetch failed)",
  });
  const [mended] = update(broken, {
    type: "connection",
    stream: "this chat",
    health: { kind: "healthy" },
  });
  assert.deepEqual(mended.connection, {});
});

test("an event with the wrong kind or a bad payload field names it", () => {
  const event = { sessionId: "s", seq: 1, createdAt: 1 };
  const kind = message(() =>
    contract.parseStoredEvent({ ...event, kind: "nope", payload: {} }),
  );
  assert.match(kind, /kind/);
  assert.match(kind, /nope/);
  assert.match(kind, /message_appended/);
  const payload = message(() =>
    contract.parseStoredEvent({
      ...event,
      kind: "archive_changed",
      payload: { archived: "yes" },
    }),
  );
  assert.match(payload, /archived/);
  assert.match(payload, /boolean/);
});

test("the body limit comes from the generated contract, not a copy", () => {
  assert.equal(contract.limits.maxBodyBytes, 1_048_576);
  const schema = contract.contractSchema as unknown as Record<string, unknown>;
  assert.equal(schema["x-maxBodyBytes"], contract.limits.maxBodyBytes);
});

test("a stream that reconnects and stays idle clears its interruption notice", async () => {
  let calls = 0;
  const fakeFetch = (async () => {
    calls += 1;
    if (calls === 1) throw new TypeError("fetch failed");
    return openStream(": keep-alive\n\n");
  }) as unknown as typeof fetch;
  const subscription = chatStream<Seen>({
    key: "idle",
    endpoint: "/sessions/events",
    token: "t",
    after: "0",
    decode: () => ({ type: "event" }),
    health: (health) => ({
      type: "connection",
      text: health.kind === "problem" ? health.text : null,
    }),
    fetch: fakeFetch,
    retryMs: 1,
  });
  const seen: Seen[] = [];
  const stop = subscription.subscribe((msg) => seen.push(msg), {} as never);
  await new Promise((resolve) => setTimeout(resolve, 60));
  stop();
  const texts = seen.filter((m) => m.type === "connection").map((m) => m.text);
  assert.match(String(texts[0]), /could not reach the server/);
  assert.equal(
    texts.at(-1),
    null,
    "opening a healthy stream clears the notice",
  );
  assert.equal(
    seen.filter((m) => m.type === "event").length,
    0,
    "no event was needed",
  );
});

test("switching chats drops the notice of the chat's own stream", () => {
  const model = {
    ...initial("t"),
    sessions: [],
    connection: {
      "this chat": "could not reach the server",
      "the chat list": "x",
    },
  };
  const [next] = update(model, { type: "select", id: "other" });
  assert.deepEqual(next.connection, { "the chat list": "x" });
});
