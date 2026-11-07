import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import * as contract from "../src/generated/contract.ts";
const fixtures: ReadonlyArray<{ type: string; value: unknown }> = JSON.parse(
  readFileSync(
    new URL("../src/generated/fixtures.json", import.meta.url),
    "utf8",
  ),
);
test("every Rust HTTP body, reply and event fixture passes its generated decoder", () => {
  for (const fixture of fixtures) {
    const parse = (
      contract as unknown as Record<string, (v: unknown) => unknown>
    )[`parse${fixture.type}`];
    assert.ok(parse, fixture.type);
    assert.deepEqual(parse(fixture.value), fixture.value);
  }
});
const body = {
  sessionId: "s",
  messageId: "00000000-0000-4000-8000-000000000001",
  content: "hello",
};
test("HTTP bodies reject missing fields, wrong types and envelope fields", () => {
  const { messageId, ...missing } = body;
  for (const invalid of [
    missing,
    { ...body, messageId: 1 },
    { ...body, content: false },
    { ...body, requestId: 1 },
    { ...body, version: 2 },
    { ...body, operation: "sendMessage" },
    { ...body, machineId: "host" },
  ])
    assert.throws(() => contract.parseSendMessage(invalid));
});
test("closed enums reject unknown values; models and efforts are checked by the server's catalog", () => {
  assert.throws(() =>
    contract.parseListSessionsResponse({
      models: [{ model: "x", label: "x", agent: "gemini", efforts: [] }],
      projects: [],
      sessions: [],
    }),
  );
  assert.throws(() => contract.parseDeliveryState("delivered"));
  // The catalog lives in the server's tuning crate, so the contract carries plain strings.
  contract.parseSetModel({ sessionId: "s", model: "future", effort: "high" });
});
test("oversized bodies and unsafe cursors fail decoding", () => {
  assert.throws(() =>
    contract.parseSendMessage({ ...body, content: "x".repeat(65537) }),
  );
  assert.throws(() =>
    contract.parseErrorInfo({ code: "x", message: "x".repeat(1_048_576) }),
  );
  for (const value of [-1, Number.MAX_SAFE_INTEGER + 1, "1"])
    assert.throws(() => contract.parseSequence(value));
});
test("typed errors have no transport envelope", () => {
  assert.deepEqual(
    contract.parseErrorInfo({ code: "invalid_request", message: "bad UUID" }),
    { code: "invalid_request", message: "bad UUID" },
  );
  assert.throws(() => contract.parseErrorInfo({ code: "invalid_request" }));
});
test("application events are closed; Claude payloads stay extensible", () => {
  const event = {
    sessionId: "s",
    seq: 1,
    createdAt: 1,
    kind: "delivery_changed",
    payload: { uuid: "m", delivery_state: "future", retry_count: null },
  };
  assert.throws(() => contract.parseStoredEvent(event));
  assert.doesNotThrow(() =>
    contract.parseStoredEvent({
      ...event,
      kind: "agent_event",
      payload: { type: "future", extra: 42 },
    }),
  );
});
