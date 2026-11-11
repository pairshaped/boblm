import { test } from "node:test";
import assert from "node:assert/strict";
import { initial, update, program, type Model } from "../src/app.ts";
const question = {
  id: "q1",
  sessionId: "s",
  question: "Which color?",
  state: "pending" as const,
  choices: ["Blue", "Red"],
  answer: null,
  createdAt: 1,
  answeredAt: null,
};
const base: Model = { ...initial("t"), active: "s", questions: [question] };
test("question answers route to their origin chat and cannot be submitted twice in flight", () => {
  const [open] = update(
    { ...base, drawerOpen: true, drawerTab: "agents" },
    { type: "openQuestions" },
  );
  assert.equal(open.questionsOpen, true);
  assert.equal(open.drawerOpen, true);
  const [sending, effects] = update(open, {
    type: "answerQuestion",
    id: "q1",
    answer: "Green",
  });
  assert.deepEqual(
    effects[0] && "body" in effects[0] ? effects[0].body : null,
    { sessionId: "s", questionId: "q1", answer: "Green" },
  );
  assert.deepEqual(
    update(sending, { type: "answerQuestion", id: "q1", answer: "Red" })[1],
    [],
  );
  const [, otherEffects] = update(
    { ...base, active: "t" },
    { type: "answerQuestion", id: "q1", answer: "Blue" },
  );
  assert.deepEqual(
    otherEffects[0] && "body" in otherEffects[0] ? otherEffects[0].body : null,
    { sessionId: "s", questionId: "q1", answer: "Blue" },
  );
});
test("lost answer replies keep the draft and never retry automatically", () => {
  const [sent] = update(
    { ...base, questionDrafts: { q1: "Green" } },
    { type: "answerQuestion", id: "q1", answer: "Green" },
  );
  const [lost, effects] = update(sent, {
    type: "reply",
    name: "answerQuestion",
    sessionId: "s",
    outcome: { kind: "uncertain" },
  });
  assert.equal(lost.questionDrafts.q1, "Green");
  assert.match(lost.questionError, /reply was lost/);
  assert.deepEqual(effects, []);
});
test("stale question snapshots cannot overwrite an answer", () => {
  const [loading] = update(base, { type: "reloadQuestions" });
  const [answered] = update(loading, {
    type: "reply",
    name: "answerQuestion",
    sessionId: "s",
    outcome: {
      kind: "replied",
      reply: {
        pendingCount: 1,
        questions: [
          { ...question, answer: "Blue", state: "answered", answeredAt: 2 },
        ],
      },
    },
  });
  const [stale] = update(answered, {
    type: "reply",
    name: "listQuestions",
    sessionId: "s",
    orderGeneration: loading.questionGeneration,
    outcome: {
      kind: "replied",
      reply: { pendingCount: 1, questions: [question] },
    },
  });
  assert.equal(stale.questions[0]?.answer, "Blue");
});

test("global list events refresh questions without an active chat", () => {
  const [loaded, effects] = update(initial("t"), {
    type: "listEvent",
    event: {
      kind: "questions_changed",
      sessionId: "other",
      seq: 1,
      createdAt: 1,
      payload: {},
    },
  });
  assert.equal(loaded.questionGeneration, 1);
  assert.ok(
    effects.some(
      (e) =>
        e.type === "request" &&
        e.name === "listQuestions" &&
        JSON.stringify(e.body) === "{}",
    ),
  );
});

test("discussing later records a separate decision and refreshes the queue without inventing an answer", () => {
  const [sent, effects] = update(base, {
    type: "discussQuestionLater",
    id: "q1",
  });
  assert.ok(
    effects.some(
      (e) => e.type === "request" && e.name === "discussQuestionLater",
    ),
  );
  assert.ok(
    !effects.some((e) => e.type === "request" && e.name === "answerQuestion"),
  );
  const [settled, reload] = update(sent, {
    type: "reply",
    name: "discussQuestionLater",
    sessionId: "s",
    outcome: {
      kind: "replied",
      reply: {
        pendingCount: 12,
        questions: [{ ...question, state: "discuss_later" }],
      },
    },
  });
  assert.equal(settled.questions[0]?.answer, null);
  assert.equal(settled.questionPendingCount, 12);
  assert.ok(
    reload.some((e) => e.type === "request" && e.name === "listQuestions"),
  );
});

test("answer labels submit immediately and only custom text needs Submit", async () => {
  const { JSDOM } = await import("jsdom");
  const { mountProgram } = await import("@pairshaped/hypertea/testing");
  const { questionsContent } = await import("../src/questions.ts");
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
  const handle = mountProgram({
    flags: {
      ...base,
      active: "other",
      questionsOpen: true,
      questionPendingCount: 73,
    },
    node: dom.window.document.querySelector<HTMLElement>("#app")!,
    program,
  });
  try {
    await handle.settle();
    const doc = dom.window.document;
    assert.ok(
      ![...doc.querySelectorAll("button")].some(
        (b) =>
          b.textContent === "Reload questions" || b.textContent === "Try again",
      ),
    );
    assert.ok(doc.querySelector("#questions-dialog"));
    const radios = doc.querySelectorAll<HTMLInputElement>("input[type=radio]");
    assert.equal(radios.length, 4);
    assert.deepEqual(
      [...radios].map((r) => r.parentElement?.textContent),
      ["Blue", "Red", "Discuss later", "Type a different answer"],
    );
    assert.match(
      doc.querySelector(".question-footer")!.textContent!,
      /72 waiting/,
    );
    handle.dispatch({
      type: "reply",
      name: "listQuestions",
      sessionId: "s",
      orderGeneration: 0,
      outcome: {
        kind: "replied",
        reply: { pendingCount: 1, questions: [question] },
      },
    });
    await handle.settle();
    assert.equal(
      doc.querySelector(".question-footer"),
      null,
      "the current question alone has no waiting line",
    );
    assert.equal(doc.querySelector("textarea"), null);
    const submitButton = () =>
      [...doc.querySelectorAll<HTMLButtonElement>("button")].find(
        (b) => b.textContent === "Submit",
      );
    assert.equal(submitButton(), undefined);
    radios[0]!.parentElement!.querySelector<HTMLElement>("span")!.click();
    await handle.settle();
    const answers = () =>
      handle
        .effects()
        .filter(
          (e) =>
            e.effect.type === "request" && e.effect.name === "answerQuestion",
        );
    assert.equal(answers().length, 1);
    assert.equal(
      (answers()[0]!.effect as { body: { answer: string } }).body.answer,
      "Blue",
    );
    handle.dispatch({
      type: "reply",
      name: "answerQuestion",
      sessionId: "s",
      outcome: { kind: "uncertain" },
    });
    await handle.settle();
    assert.equal(
      radios[0]!.checked,
      false,
      "a lost reply allows an explicit retry by choosing the same answer",
    );
    radios[0]!.parentElement!.querySelector<HTMLElement>("span")!.click();
    await handle.settle();
    assert.equal(answers().length, 2);
    handle.dispatch({
      type: "reply",
      name: "answerQuestion",
      sessionId: "s",
      outcome: { kind: "uncertain" },
    });
    await handle.settle();
    radios[2]!.parentElement!.querySelector<HTMLElement>("span")!.click();
    await handle.settle();
    assert.equal(doc.querySelector("textarea"), null);
    assert.equal(submitButton(), undefined);
    assert.ok(
      handle
        .effects()
        .some(
          (e) =>
            e.effect.type === "request" &&
            e.effect.name === "discussQuestionLater",
        ),
    );
    handle.dispatch({
      type: "reply",
      name: "discussQuestionLater",
      sessionId: "s",
      outcome: { kind: "uncertain" },
    });
    await handle.settle();
    const beforeCustom = answers().length;
    radios[3]!.click();
    await handle.settle();
    const custom = doc.querySelector<HTMLTextAreaElement>("textarea")!;
    assert.ok(custom);
    assert.equal(
      answers().length,
      beforeCustom,
      "selecting the custom option doesn't submit",
    );
    custom.value = "Green";
    custom.dispatchEvent(new dom.window.Event("input", { bubbles: true }));
    await handle.settle();
    const submit = [...doc.querySelectorAll<HTMLButtonElement>("button")].find(
      (b) => b.textContent === "Submit",
    )!;
    assert.equal(submit.disabled, false);
    submit.click();
    await handle.settle();
    assert.ok(
      handle
        .effects()
        .some(
          (e) =>
            e.effect.type === "request" &&
            e.effect.name === "answerQuestion" &&
            (e.effect.body as { answer: string }).answer === "Green",
        ),
    );
  } finally {
    handle.stop();
    dom.window.close();
  }
});
