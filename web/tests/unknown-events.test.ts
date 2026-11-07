import { test } from "node:test";
import assert from "node:assert/strict";
import {
  decodeChatRecord,
  decodeListRecord,
  initial,
  update,
} from "../src/app.ts";

const list = (seq: number, kind: string, payload: unknown = {}) => ({
  event: "sessionListEvent",
  id: String(seq),
  data: JSON.stringify({ seq, kind, sessionId: "s", createdAt: 1, payload }),
});
const chat = (seq: number, kind: string, payload: unknown = {}) => ({
  event: "sessionEvent",
  id: String(seq),
  data: JSON.stringify({ seq, kind, sessionId: "s", createdAt: 1, payload }),
});

test("an unknown chat-list event kind is skipped and the next known event still applies", () => {
  let model = initial("t");
  [model] = update(model, decodeListRecord(list(1, "from_a_newer_server")));
  assert.equal(model.listCursor, 1, "the cursor moves past the unknown event");
  const next = decodeListRecord(list(2, "subagents_changed"));
  assert.equal(next.type, "listEvent");
  [model] = update(model, next);
  assert.equal(model.listCursor, 2);
});

test("a known list event with a bad payload is still refused", () => {
  assert.throws(() => decodeListRecord(list(1, "renamed", { nope: 1 })));
});

test("an unknown chat event kind is skipped and advances that chat's cursor", () => {
  let model = initial("t");
  [model] = update(
    model,
    decodeChatRecord(chat(7, "from_a_newer_server"), "s", "this chat"),
  );
  assert.equal(model.cursors.s, 7);
  const known = decodeChatRecord(chat(8, "session_stopped"), "s", "this chat");
  assert.equal(known.type, "event");
});
