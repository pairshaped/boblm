import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { initial, update, type Model } from "../src/app.ts";
import { reloadPage, readQuestionDrafts } from "../src/browser-reload.ts";

test("page reload preserves drafts, question answers, current chat and in-flight messages", () => {
  const storage = new JSDOM("", { url: "http://localhost" }).window
    .localStorage;
  const model: Model = {
    ...initial("t"),
    active: "s",
    drafts: { s: "Unsent draft" },
    questionDrafts: { q: "Custom answer" },
    questionChoices: { q: null },
    pending: { s: { id: "message-id", content: "In flight", attachments: [] } },
  };
  let count = 0;
  reloadPage(model, storage, () => count++);
  assert.equal(count, 1);
  assert.deepEqual(JSON.parse(storage.getItem("boblm:drafts")!), model.drafts);
  assert.equal(storage.getItem("boblm:lastChat"), "s");
  assert.deepEqual(
    JSON.parse(storage.getItem("boblm:uncertain")!),
    model.pending,
  );
  assert.deepEqual(readQuestionDrafts(storage), {
    questionDrafts: model.questionDrafts,
    questionChoices: model.questionChoices,
  });
});
test("storage failure prevents reload and invalid saved question state is ignored", () => {
  let reloaded = false;
  assert.throws(() =>
    reloadPage(
      initial("t"),
      {
        setItem: () => {
          throw Error("storage is full");
        },
      } as unknown as Storage,
      () => {
        reloaded = true;
      },
    ),
  );
  assert.equal(reloaded, false);
  const storage = new JSDOM("", { url: "http://localhost" }).window
    .localStorage;
  storage.setItem("boblm:questionDrafts", "broken JSON");
  assert.deepEqual(readQuestionDrafts(storage), {
    questionDrafts: {},
    questionChoices: {},
  });
});
test("reload messages cannot affect another chat or discard an unfinished upload", () => {
  const model = { ...initial("t"), active: "s" };
  assert.deepEqual(
    update(model, { type: "reloadPage", sessionId: "other" })[1],
    [],
  );
  assert.equal(
    update(model, { type: "reloadPage", sessionId: "s" })[1][0]?.type,
    "reloadPage",
  );
  const uploading: Model = {
    ...model,
    attachments: {
      s: [
        {
          id: "upload",
          name: "image.png",
          contentType: "image/png",
          size: 1,
          state: "uploading",
        },
      ],
    },
  };
  const [blocked, effects] = update(uploading, {
    type: "reloadPage",
    sessionId: "s",
  });
  assert.deepEqual(effects, []);
  assert.match(blocked.error, /attachment/);
});
