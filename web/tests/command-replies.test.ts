import { test } from "node:test";
import assert from "node:assert/strict";
import { postCommand, parseCommandReply } from "../src/http.ts";

test("project commands decode project and empty replies as successful results", async () => {
  const project = {
    projectId: "20000000-0000-4000-8000-000000000001",
    name: "alpha",
    path: "/home/me/alpha",
    createdAt: 1,
  };
  for (const [name, reply] of [
    ["addProject", project],
    ["removeProject", {}],
    ["reorderProjects", {}],
    ["reorderChats", {}],
  ] as const) {
    const result = await postCommand(
      `/commands/${name}`,
      {},
      "fixture",
      (v) => parseCommandReply(name, v),
      async () => new Response(JSON.stringify(reply), { status: 200 }),
    );
    assert.deepEqual(result, { kind: "replied", reply });
  }
  assert.throws(() => parseCommandReply("addProject", {}));
  assert.throws(() =>
    parseCommandReply("reorderProjects", { unexpected: true }),
  );
});
