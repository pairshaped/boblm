import { h, bindEvents } from "@pairshaped/hypertea/program";
import type { Model, Msg } from "./app.ts";
const on = bindEvents<Msg>();

export function questionsContent(model: Model) {
  const subagentLabel = (sessionId: string): string => {
    const session = model.sessions.find((s) => s.sessionId === sessionId);
    const parent = model.sessions.find(
      (s) =>
        s.sessionId === (session?.rootSessionId ?? session?.parentSessionId),
    );
    return session?.parentSessionId
      ? `Subagent: ${session.name.trim() || sessionId}${parent ? ` · ${parent.name}` : ""}`
      : session?.name.trim() || sessionId;
  };
  const questions = model.questions;
  const pending = questions.filter((q) => q.state === "pending");
  const answered = questions.filter((q) => q.state === "answered");
  const totalPending = Math.max(model.questionPendingCount, pending.length);
  const waiting = Math.max(0, totalPending - 1);
  const saving = Object.keys(model.questionSending).length > 0;
  return h(
    "div",
    { class: "subagents-body" },
    model.questionError
      ? h(
          "div",
          {},
          h("p", { role: "alert", class: "error" }, model.questionError),
          h(
            "button",
            {
              type: "button",
              onClick: on.clicked({ type: "reloadQuestions" }),
            },
            "Try again",
          ),
        )
      : null,
    totalPending === 0
      ? h("p", { role: "status" }, "No unanswered questions")
      : null,
    pending.slice(0, 1).map((q) => {
      const selected = model.questionChoices[q.id];
      const discussion = selected === "discuss_later";
      const other =
        !discussion && (q.choices.length === 0 || selected === null);
      const answer = model.questionDrafts[q.id] ?? "";
      return h(
        "section",
        {
          key: q.id,
          class: "question current-question",
          id: `question-${q.id}`,
        },
        subagentLabel(q.sessionId)
          ? h("p", { class: "question-subagent" }, subagentLabel(q.sessionId))
          : null,
        h(
          "fieldset",
          { disabled: saving },
          h("legend", {}, q.question),
          q.choices.map((choice, index) =>
            h(
              "label",
              { class: "question-option" },
              h("input", {
                type: "radio",
                name: `answer-${q.id}`,
                value: String(index),
                controlledChecked: selected === index,
                onChange: on.changed({
                  type: "questionChoice",
                  id: q.id,
                  choice: index,
                }),
              }),
              h("span", {}, choice),
            ),
          ),
          h(
            "label",
            { class: "question-option" },
            h("input", {
              type: "radio",
              name: `answer-${q.id}`,
              value: "discuss_later",
              controlledChecked: discussion,
              onChange: on.changed({
                type: "questionChoice",
                id: q.id,
                choice: "discuss_later",
              }),
            }),
            h("span", {}, "Discuss later"),
          ),
          h(
            "label",
            { class: "question-option" },
            h("input", {
              type: "radio",
              name: `answer-${q.id}`,
              value: "other",
              controlledChecked: other,
              onChange: on.changed({
                type: "questionChoice",
                id: q.id,
                choice: null,
              }),
            }),
            h("span", {}, "Type a different answer"),
          ),
          other
            ? h("textarea", {
                "aria-label": `Answer: ${q.question}`,
                placeholder: "Type your answer",
                autocomplete: "off",
                value: model.questionDrafts[q.id] ?? "",
                readOnly: saving,
                onInput: on.inputChanged((text) => ({
                  type: "questionDraft",
                  id: q.id,
                  text,
                })),
              })
            : null,
        ),
        other
          ? h(
              "div",
              { class: "question-actions" },
              h(
                "button",
                {
                  type: "button",
                  disabled: saving || !answer.trim(),
                  onClick: on.clicked({
                    type: "answerQuestion",
                    id: q.id,
                    answer,
                  }),
                },
                saving ? "Saving…" : "Submit",
              ),
            )
          : null,
        waiting > 0
          ? h(
              "footer",
              { class: "question-footer", "aria-live": "polite" },
              `${waiting} waiting`,
            )
          : null,
      );
    }),
    totalPending > 0 && !pending.length
      ? h("p", { role: "status" }, "Loading the next question…")
      : null,
    answered.length ? h("h3", {}, "Recently answered") : null,
    answered.map((q) =>
      h(
        "section",
        { key: q.id, class: "question answered", id: `question-${q.id}` },
        subagentLabel(q.sessionId)
          ? h("p", { class: "question-subagent" }, subagentLabel(q.sessionId))
          : null,
        h("p", {}, q.question),
        h("p", {}, `Answered: ${q.answer}`),
      ),
    ),
  );
}
