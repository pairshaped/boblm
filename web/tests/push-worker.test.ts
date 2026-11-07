import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";

test("push always displays a notification, deduplicates by ID, and links the relevant question", async () => {
  const handlers: Record<string, (event: any) => void> = {};
  const shown: any[] = [];
  const opened: string[] = [];
  runInNewContext(readFileSync(new URL("../sw.js", import.meta.url), "utf8"), {
    URL,
    self: {
      location: { origin: "https://boblm.example" },
      addEventListener: (name: string, fn: any) => {
        handlers[name] = fn;
      },
      registration: {
        showNotification: (...args: any[]) => {
          shown.push(args);
          return Promise.resolve();
        },
      },
      clients: {
        matchAll: async () => [],
        openWindow: async (url: string) => {
          opened.push(url);
        },
      },
    },
  });
  let wait: Promise<unknown> = Promise.resolve();
  handlers.push!({
    data: {
      json: () => ({
        id: "n",
        title: "Question",
        body: "Which color?",
        sessionId: "s",
        questionId: "q",
      }),
    },
    waitUntil: (p: Promise<unknown>) => {
      wait = p;
    },
  });
  await wait;
  assert.equal(shown[0][1].tag, "n");
  assert.equal(new URL(shown[0][1].data.url).searchParams.get("question"), "q");
  handlers.notificationclick!({
    notification: { close: () => {}, data: shown[0][1].data },
    waitUntil: (p: Promise<unknown>) => {
      wait = p;
    },
  });
  await wait;
  assert.equal(new URL(opened[0]!).searchParams.get("chat"), "s");
  handlers.push!({
    data: {
      json: () => {
        throw Error("broken");
      },
    },
    waitUntil: (p: Promise<unknown>) => {
      wait = p;
    },
  });
  await wait;
  assert.equal(
    shown[1][0],
    "BobLM",
    "malformed payloads still produce a visible notification",
  );
});
