import { connectToServer } from "../src/connection.ts";
import { test } from "node:test";
import assert from "node:assert/strict";
import { initial, update, subscriptions } from "../src/app.ts";

test("connecting waits for validation before saving the token and ignores repeated submits", () => {
  const [pending, effects] = update(initial(""), {
    type: "login",
    token: " fixture ",
  });
  assert.equal(pending.connectionPending, true);
  assert.equal(pending.token, "fixture");
  assert.deepEqual(effects, [
    {
      type: "connect",
      token: "fixture",
      generation: pending.connectionGeneration,
    },
  ]);
  assert.equal(
    effects.some((e) => e.type === "persist"),
    false,
  );
  assert.deepEqual(update(pending, { type: "login", token: "other" })[1], []);
  const [connected, saved] = update(pending, {
    type: "connectionReply",
    generation: pending.connectionGeneration,
    outcome: {
      kind: "replied",
      reply: { sessions: [], projects: [], models: [] },
    },
  });
  assert.equal(connected.connectionPending, false);
  assert.equal(connected.token, "fixture");
  assert.equal(saved[0]?.type, "persist");
  assert.ok(saved.some((e) => e.type === "persist" && e.token === "fixture"));
});

test("a rejected or lost connection returns to token entry without saving the rejected token", () => {
  for (const outcome of [
    {
      kind: "rejected" as const,
      error: { code: "unauthorized", message: "The token was not accepted." },
    },
    { kind: "uncertain" as const, reason: "The connection was lost." },
  ]) {
    const [pending] = update(initial(""), { type: "login", token: "bad" });
    const [failed, effects] = update(pending, {
      type: "connectionReply",
      generation: pending.connectionGeneration,
      outcome,
    });
    assert.equal(failed.token, "");
    assert.equal(failed.connectionPending, false);
    assert.match(failed.error, /Could not connect/);
    assert.deepEqual(effects, []);
    assert.equal(
      update(failed, { type: "login", token: "retry" })[0].connectionPending,
      true,
    );
  }
});

test("stale authentication callbacks cannot finish a later attempt and refresh does not compete", () => {
  const [first] = update(initial(""), { type: "login", token: "first" });
  const [failed] = update(first, {
    type: "connectionReply",
    generation: first.connectionGeneration,
    outcome: { kind: "uncertain" },
  });
  const [second] = update(failed, { type: "login", token: "second" });
  const [ignored, effects] = update(second, {
    type: "connectionReply",
    generation: first.connectionGeneration,
    outcome: {
      kind: "replied",
      reply: { sessions: [], projects: [], models: [] },
    },
  });
  assert.equal(ignored, second);
  assert.deepEqual(effects, []);
  assert.deepEqual(update(second, { type: "refresh" })[1], []);
  assert.deepEqual(
    subscriptions(second).map((s) => s.key),
    subscriptions(initial("")).map((s) => s.key),
  );
});

test("authentication sends the entered token, independent of saved transport state", async () => {
  let authorization: string | null = null;
  const request: typeof fetch = async (_url, init) => {
    authorization = new Headers(init?.headers).get("Authorization");
    return new Response(
      JSON.stringify({ sessions: [], projects: [], models: [] }),
      { headers: { "Content-Type": "application/json" } },
    );
  };
  const result = await connectToServer("entered-token", request);
  assert.equal(authorization, "Bearer entered-token");
  assert.equal(result.kind, "replied");
});

test("a stalled connection or stalled response body times out and can be retried", async () => {
  for (const body of [false, true]) {
    let aborted = false;
    const request: typeof fetch = async (_url, init) => {
      init?.signal?.addEventListener("abort", () => {
        aborted = true;
      });
      if (!body) return new Promise(() => {});
      return new Response(new ReadableStream({ start() {} }), {
        headers: { "Content-Type": "application/json" },
      });
    };
    const result = await connectToServer("entered-token", request, 10);
    assert.equal(result.kind, "uncertain");
    if (result.kind === "uncertain") assert.match(result.reason!, /timed out/);
    assert.equal(aborted, true);
  }
});
