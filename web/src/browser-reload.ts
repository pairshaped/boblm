import type { Model } from "./app.ts";

/** Save the current draft state before crossing the document-reload boundary. */
export function reloadPage(model: Model, storage: Storage, reload: () => void) {
  storage.setItem("boblm:drafts", JSON.stringify(model.drafts));
  storage.setItem("boblm:attachments", JSON.stringify(model.attachments));
  storage.setItem(
    "boblm:uncertain",
    JSON.stringify({ ...model.uncertain, ...model.pending }),
  );
  storage.setItem(
    "boblm:questionDrafts",
    JSON.stringify({
      drafts: model.questionDrafts,
      choices: model.questionChoices,
    }),
  );
  if (model.active) storage.setItem("boblm:lastChat", model.active);
  reload();
}

export function readQuestionDrafts(
  storage: Storage,
): Pick<Model, "questionDrafts" | "questionChoices"> {
  const empty = { questionDrafts: {}, questionChoices: {} };
  try {
    const value: unknown = JSON.parse(
      storage.getItem("boblm:questionDrafts") ?? "null",
    );
    if (!value || typeof value !== "object") return empty;
    const { drafts, choices } = value as {
      drafts?: unknown;
      choices?: unknown;
    };
    if (
      !drafts ||
      typeof drafts !== "object" ||
      Array.isArray(drafts) ||
      !choices ||
      typeof choices !== "object" ||
      Array.isArray(choices)
    )
      return empty;
    return {
      questionDrafts: Object.fromEntries(
        Object.entries(drafts).filter(
          (entry): entry is [string, string] => typeof entry[1] === "string",
        ),
      ),
      questionChoices: Object.fromEntries(
        Object.entries(choices).filter(
          (entry): entry is [string, number | "discuss_later" | null] =>
            entry[1] === null ||
            entry[1] === "discuss_later" ||
            (typeof entry[1] === "number" &&
              Number.isSafeInteger(entry[1]) &&
              entry[1] >= 0),
        ),
      ),
    };
  } catch {
    return empty;
  }
}
