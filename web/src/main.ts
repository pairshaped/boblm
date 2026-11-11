import { subagentConversationSubscription } from "./subagent-conversation.ts";
import { historyResumeSubscription } from "./history-resume.ts";
import { connectToServer } from "./connection.ts";
import { reloadPage, readQuestionDrafts } from "./browser-reload.ts";
import {
  pushSubscription,
  presenceSubscription,
  enablePush,
  disablePush,
} from "./push.ts";
import { listenForProjectRename } from "./project-press.ts";
import { limitTabStops } from "./tab-stops.ts";
import { listenForComposerFocus } from "./composer-focus.ts";
import { fitComposer } from "./composer-size.ts";
import { speak, stopSpeaking } from "./speech.ts";
import { lightboxSubscription } from "./lightbox.ts";
import {
  historyScroll,
  restoreHistoryAnchor,
  scrollSubscription,
} from "./scroll.ts";
import {
  emptySubscription,
  mountProgram,
  type ProgramSubscription,
} from "@pairshaped/hypertea/program";
import {
  program,
  initial,
  subscriptions,
  shortcutTarget,
  type Model,
  type Msg,
  type Effect,
} from "./app.ts";
import {
  postCommand,
  putUpload,
  fetchUpload,
  parseCommandReply,
} from "./http.ts";
import {
  parseAttachmentInfo,
  parseChatHistoryPage,
  limits,
} from "./generated/contract.ts";
const root = document.querySelector<HTMLElement>("#app");
if (!root) throw new Error("Missing app root");
let token = localStorage.getItem("boblm:token") ?? "";
let drafts: Readonly<Record<string, string>> = {};
try {
  const value: unknown = JSON.parse(
    localStorage.getItem("boblm:drafts") ?? "{}",
  );
  if (value && typeof value === "object" && !Array.isArray(value))
    drafts = Object.fromEntries(
      Object.entries(value).filter(
        (entry): entry is [string, string] => typeof entry[1] === "string",
      ),
    );
} catch {
  /* A damaged local draft cannot prevent opening the app. */
}
let uncertain: NonNullable<Parameters<typeof initial>[3]> = {};
try {
  const value: unknown = JSON.parse(
    localStorage.getItem("boblm:uncertain") ?? "{}",
  );
  if (value && typeof value === "object" && !Array.isArray(value))
    uncertain = Object.fromEntries(
      Object.entries(value).filter(
        ([, held]) =>
          held &&
          typeof held.id === "string" &&
          typeof held.content === "string" &&
          Array.isArray(held.attachments) &&
          held.attachments.every((id: unknown) => typeof id === "string"),
      ),
    ) as typeof uncertain;
} catch {
  /* A damaged record only loses the retry; the draft text is kept separately. */
}
let lastChoice: Parameters<typeof initial>[4] = null;
try {
  const value: unknown = JSON.parse(
    localStorage.getItem("boblm:choice") ?? "null",
  );
  if (
    value &&
    typeof value === "object" &&
    typeof (value as { model?: unknown }).model === "string"
  ) {
    const { model, effort } = value as { model: string; effort?: unknown };
    lastChoice = { model, effort: typeof effort === "string" ? effort : null };
  }
} catch {
  /* A damaged record only loses the remembered choice. */
}
let collapsed: Parameters<typeof initial>[5] = {};
try {
  const value: unknown = JSON.parse(
    localStorage.getItem("boblm:collapsed") ?? "{}",
  );
  if (value && typeof value === "object" && !Array.isArray(value))
    collapsed = Object.fromEntries(
      Object.keys(value).map((id) => [id, true as const]),
    );
} catch {
  /* A damaged record only unfolds the projects. */
}
let dismissed: Parameters<typeof initial>[6] = {};
try {
  const value: unknown = JSON.parse(
    localStorage.getItem("boblm:dismissed") ?? "{}",
  );
  if (value && typeof value === "object" && !Array.isArray(value))
    dismissed = Object.fromEntries(
      Object.keys(value).map((id) => [id, true as const]),
    );
} catch {
  /* A damaged record only shows hidden messages in the banner again. */
}
const deepLink = new URLSearchParams(location.search);
const lastChat = deepLink.get("chat") ?? localStorage.getItem("boblm:lastChat");
if (deepLink.has("chat"))
  history.replaceState(null, "", location.pathname + location.hash);
let cachedList: Parameters<typeof initial>[9] = null;
try {
  const saved = localStorage.getItem("boblm:list");
  // The same checks as a reply from the server, so a damaged or outdated copy is simply ignored.
  if (saved)
    cachedList = parseCommandReply(
      "listSessions",
      JSON.parse(saved),
    ) as typeof cachedList;
} catch {
  /* Showing the last list early is a convenience; the real list is requested anyway. */
}
let attachments: Parameters<typeof initial>[2] = {};
try {
  const value: unknown = JSON.parse(
    localStorage.getItem("boblm:attachments") ?? "{}",
  );
  if (value && typeof value === "object" && !Array.isArray(value))
    attachments = Object.fromEntries(
      Object.entries(value).filter(
        ([, list]) =>
          Array.isArray(list) &&
          list.every(
            (a) =>
              a &&
              typeof a.id === "string" &&
              typeof a.name === "string" &&
              typeof a.contentType === "string" &&
              typeof a.size === "number" &&
              a.state === "ready",
          ),
      ),
    ) as typeof attachments;
} catch {
  /* A damaged local list only means the files must be attached again. */
}
function shortcutSubscription(model: Model): ProgramSubscription<Msg> {
  if (!model.token || model.connectionPending)
    return emptySubscription<Msg>("slot:shortcuts");
  return {
    key: "shortcuts",
    subscribe: (dispatch) => {
      const press = (event: KeyboardEvent) => {
        const number = shortcutTarget(event);
        if (number === null) return;
        event.preventDefault();
        dispatch({ type: "jump", number });
      };
      window.addEventListener("keydown", press);
      return () => window.removeEventListener("keydown", press);
    },
  };
}
/** Files chosen in the composer, kept until sent or removed; the model holds only metadata. */
const chosenFiles = new Map<string, File>();
const run = async (dispatch: (msg: Msg) => void, effect: Effect) => {
  if (effect.type === "connect") {
    const outcome = await connectToServer(effect.token);
    dispatch({
      type: "connectionReply",
      generation: effect.generation,
      outcome,
    });
    return;
  }

  if (effect.type === "historyRequest") {
    historyScroll.controller?.abort();
    historyScroll.controller = new AbortController();
    const controller = historyScroll.controller;
    historyScroll.anchor = null;
    const messages = document.querySelector<HTMLElement>("#messages");
    if (
      effect.direction !== "latest" &&
      messages?.dataset.sessionId === effect.sessionId
    ) {
      const bounds = messages.getBoundingClientRect();
      const row = [
        ...messages.querySelectorAll<HTMLElement>("[data-row-key]"),
      ].find((el) => el.getBoundingClientRect().bottom > bounds.top);
      if (row?.dataset.rowKey)
        historyScroll.anchor = {
          sessionId: effect.sessionId,
          generation: effect.generation,
          key:
            effect.direction === "newer"
              ? ((JSON.parse(row.dataset.rowKeys ?? "[]") as string[]).at(-1) ??
                row.dataset.rowKey)
              : row.dataset.rowKey,
          offset: row.getBoundingClientRect().top - bounds.top,
        };
    }
    const outcome = await postCommand(
      "/commands/chatHistory",
      {
        sessionId: effect.sessionId,
        ...(effect.direction === "older"
          ? { before: effect.position }
          : effect.direction === "newer"
            ? { after: effect.position }
            : {}),
      },
      token,
      parseChatHistoryPage,
      (url, init) => fetch(url, { ...init, signal: controller.signal }),
    );
    if (historyScroll.controller === controller)
      historyScroll.controller = null;
    dispatch({
      type: "historyReply",
      sessionId: effect.sessionId,
      generation: effect.generation,
      outcome,
    });
    return;
  }
  if (effect.type === "historyScroll") {
    requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        const selector =
          effect.target === "subagent" ? "#subagent-messages" : "#messages";
        const messages = document.querySelector<HTMLElement>(selector);
        if (messages?.dataset.sessionId !== effect.sessionId) return;
        if (effect.latest) messages.scrollTop = messages.scrollHeight;
        else restoreHistoryAnchor(messages);
        if (historyScroll.anchor?.generation === effect.generation)
          historyScroll.anchor = null;
        messages.dispatchEvent(new Event("scroll"));
      }),
    );
    return;
  }
  if (effect.type === "persistList") {
    try {
      localStorage.setItem("boblm:list", JSON.stringify(effect.list));
    } catch {
      /* Remembering the list is a convenience. */
    }
    return;
  }
  if (effect.type === "persistLastChat") {
    try {
      if (effect.id) localStorage.setItem("boblm:lastChat", effect.id);
      else localStorage.removeItem("boblm:lastChat");
    } catch {
      /* Remembering the open chat is a convenience. */
    }
    return;
  }
  if (effect.type === "focusLightbox") {
    // The picture is drawn just after this runs.
    requestAnimationFrame(() =>
      document.querySelector<HTMLElement>("#lightbox-close")?.focus(),
    );
    return;
  }
  if (effect.type === "focusImage") {
    requestAnimationFrame(() =>
      document
        .querySelector<HTMLElement>(
          `[data-image-open="${CSS.escape(effect.id)}"]`,
        )
        ?.focus(),
    );
    return;
  }
  if (effect.type === "speak") {
    // Started inside the click that asked for it, which iPhone Safari requires for audio.
    try {
      speak(effect.text, (failed) =>
        dispatch({ type: "speechEnded", key: effect.key, failed }),
      );
    } catch {
      dispatch({ type: "speechEnded", key: effect.key, failed: true });
    }
    return;
  }
  if (effect.type === "stopSpeech") {
    stopSpeaking();
    return;
  }
  if (effect.type === "copy") {
    const source = effect.source;
    // A text button (code blocks) swaps its words; an icon button only changes class.
    const words = source?.classList.contains("copy-code")
      ? source.textContent
      : null;
    const done = (ok: boolean) => {
      if (!source) return;
      if (words !== null) source.textContent = ok ? "Copied" : "Copy failed";
      else source.classList.add(ok ? "copied" : "copy-failed");
      setTimeout(() => {
        if (words !== null) source.textContent = words;
        else source.classList.remove("copied", "copy-failed");
      }, 1500);
    };
    navigator.clipboard
      .writeText(effect.text)
      .then(() => done(true))
      .catch(() => done(false));
    return;
  }
  if (effect.type === "persistDismissed") {
    try {
      localStorage.setItem("boblm:dismissed", JSON.stringify(effect.dismissed));
    } catch {
      /* Remembering hidden messages is a convenience. */
    }
    return;
  }
  if (effect.type === "persistCollapsed") {
    try {
      localStorage.setItem("boblm:collapsed", JSON.stringify(effect.collapsed));
    } catch {
      /* Remembering which projects are folded is a convenience. */
    }
    return;
  }
  if (effect.type === "persistChoice") {
    try {
      localStorage.setItem("boblm:choice", JSON.stringify(effect.choice));
    } catch {
      /* Remembering the last model is a convenience; creating the chat goes on. */
    }
    return;
  }
  if (effect.type === "confirmDelete") {
    if (
      window.confirm(
        `Delete “${effect.name}”? Its BobLM history, schedules, and uploaded files will be permanently removed. Project files and the agent's own transcript will remain.`,
      )
    )
      dispatch({ type: "deleteChat", sessionId: effect.sessionId });
    return;
  }
  if (effect.type === "confirmRemoveProject") {
    const what =
      effect.chats === 0
        ? `Remove “${effect.name}” from BobLM? Its folder and files stay where they are.`
        : `Remove “${effect.name}” and its ${effect.chats} archived ${effect.chats === 1 ? "chat" : "chats"}? Their BobLM history, schedules, and uploaded files will be permanently removed. Project files and the agents' own transcripts will remain.`;
    if (window.confirm(what))
      dispatch({ type: "removeProject", projectId: effect.projectId });
    return;
  }
  if (effect.type === "toggleFullscreen") {
    try {
      if (document.fullscreenElement) await document.exitFullscreen();
      else {
        if (!document.fullscreenEnabled)
          throw new Error("This browser does not support fullscreen here");
        await document.documentElement.requestFullscreen();
      }
    } catch (error) {
      dispatch({
        type: "error",
        message: `Could not change fullscreen: ${error instanceof Error ? error.message : String(error)}. Try your browser's fullscreen control.`,
      });
    }
    return;
  }
  if (effect.type === "promptProjectRename") {
    const name = window.prompt("Project name", effect.name);
    if (name !== null && name.trim() !== effect.name)
      dispatch({ type: "renameProject", projectId: effect.projectId, name });
    return;
  }
  if (effect.type === "confirmLock") {
    if (
      window.confirm(
        "Lock BobLM? You'll need to enter your token again in this browser. Running chats will continue.",
      )
    )
      dispatch({ type: "logout" });
    return;
  }
  if (effect.type === "directoryFocus") {
    // Effects run before the view flush; focus after the dialog has been drawn.
    requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        document
          .querySelector<HTMLButtonElement>(
            effect.open ? ".directory-picker button" : "[data-browse-folders]",
          )
          ?.focus();
      }),
    );
    return;
  }
  if (effect.type === "persistUncertain") {
    try {
      localStorage.setItem("boblm:uncertain", JSON.stringify(effect.uncertain));
    } catch {
      dispatch({
        type: "error",
        message: "This browser could not save the unconfirmed send.",
      });
    }
    return;
  }
  if (effect.type === "persistAttachments") {
    try {
      localStorage.setItem(
        "boblm:attachments",
        JSON.stringify(effect.attachments),
      );
    } catch {
      dispatch({
        type: "error",
        message: "This browser could not save the attached files.",
      });
    }
    return;
  }
  if (effect.type === "upload") {
    if (effect.file) chosenFiles.set(effect.id, effect.file);
    const file = chosenFiles.get(effect.id);
    dispatch({
      type: "uploaded",
      sessionId: effect.sessionId,
      id: effect.id,
      outcome: file
        ? await putUpload(
            effect.sessionId,
            effect.id,
            file,
            token,
            parseAttachmentInfo,
          )
        : { kind: "uncertain" },
    });
    return;
  }
  if (effect.type === "discard") {
    chosenFiles.delete(effect.id);
    if (effect.url) URL.revokeObjectURL(effect.url);
    return;
  }
  if (effect.type === "thumbnail") {
    const blob =
      effect.file ?? (await fetchUpload(effect.sessionId, effect.id, token));
    if (blob)
      dispatch({
        type: "thumb",
        id: effect.id,
        url: URL.createObjectURL(blob),
      });
    return;
  }
  if (effect.type === "download") {
    const blob = await fetchUpload(effect.sessionId, effect.id, token);
    if (!blob) {
      dispatch({
        type: "error",
        message: `Could not download ${effect.name}.`,
      });
      return;
    }
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = effect.name;
    link.click();
    setTimeout(() => URL.revokeObjectURL(url), 10_000);
    return;
  }
  if (effect.type === "focusQuestions") {
    requestAnimationFrame(() =>
      document
        .querySelector<HTMLElement>(
          effect.open ? "#questions-dialog button" : "#open-questions",
        )
        ?.focus(),
    );
    return;
  }
  if (effect.type === "focusQuestion") {
    requestAnimationFrame(() => {
      const question = document.getElementById(`question-${effect.id}`);
      question?.scrollIntoView({ block: "center" });
      if (question) {
        question.tabIndex = -1;
        question.focus({ preventScroll: true });
      }
    });
    return;
  }
  if (effect.type === "togglePush") {
    try {
      if (effect.enabled) await disablePush(effect.token);
      else await enablePush(effect.token);
      dispatch({
        type: "pushState",
        state: effect.enabled ? "available" : "enabled",
      });
    } catch (error) {
      dispatch({
        type: "error",
        message:
          error instanceof Error
            ? error.message
            : "Push could not be changed. Check the connection and try again.",
      });
    }
    return;
  }
  if (effect.type === "reloadPage") {
    try {
      reloadPage(effect.model, localStorage, () => window.location.reload());
    } catch (error) {
      dispatch({
        type: "error",
        message: `Could not refresh because this browser could not save your drafts: ${error instanceof Error ? error.message : String(error)}. Free local storage or copy your drafts before refreshing manually.`,
      });
    }
    return;
  }
  if (effect.type === "persist") {
    if (token && !effect.token) {
      void disablePush(token).catch((error) =>
        console.error(
          error instanceof Error
            ? error.message
            : "Push unsubscription failed.",
        ),
      );
    }
    token = effect.token;
    try {
      localStorage.setItem("boblm:token", token);
      localStorage.setItem("boblm:drafts", JSON.stringify(effect.drafts));
    } catch {
      dispatch({
        type: "error",
        message: "This browser could not save the local draft.",
      });
    }
    return;
  }
  if (effect.type === "composer") {
    // The composer for a newly selected chat is drawn after this effect runs, so wait for
    // it (a few frames at most) before filling it and moving the cursor in.
    let frames = 0;
    const place = () => {
      const textarea = document.querySelector<HTMLTextAreaElement>("#composer");
      if (textarea && textarea.dataset.sessionId === effect.sessionId) {
        textarea.value = effect.text;
        fitComposer(textarea);
        textarea.focus();
      } else if (frames++ < 30) requestAnimationFrame(place);
    };
    queueMicrotask(place);
    return;
  }
  if (effect.type === "bottom") {
    queueMicrotask(() => {
      const list = document.querySelector<HTMLElement>("#messages");
      if (list) {
        list.scrollTop = list.scrollHeight;
        list.dispatchEvent(new Event("scroll"));
      }
    });
    return;
  }
  const outcome = await postCommand(
    `/commands/${effect.name}`,
    effect.body,
    token,
    (value) => parseCommandReply(effect.name, value),
  );
  dispatch({
    type: "reply",
    name: effect.name,
    sessionId: effect.sessionId,
    ...(effect.orderGeneration === undefined
      ? {}
      : { orderGeneration: effect.orderGeneration }),
    ...(effect.listRequest === undefined
      ? {}
      : { listRequest: effect.listRequest }),
    outcome,
  });
};
mountProgram({
  flags: {
    ...initial(
      token,
      drafts,
      attachments,
      uncertain,
      lastChoice,
      collapsed,
      dismissed,
      false,
      lastChat,
      cachedList,
    ),
    fullscreen: !!document.fullscreenElement,
    questionsOpen: deepLink.has("question"),
    ...readQuestionDrafts(localStorage),
    questionToFocus: deepLink.get("question"),
  },
  node: root,
  program: {
    ...program,
    subscriptions: (model) => [
      ...subscriptions(model),
      pushSubscription(model),
      presenceSubscription(model),
      scrollSubscription(model),
      subagentConversationSubscription(model),
      lightboxSubscription(model),
      historyResumeSubscription(model),
      shortcutSubscription(model),
      {
        key: "project-rename",
        subscribe: (dispatch) =>
          listenForProjectRename(document, (projectId) =>
            dispatch({ type: "promptProjectRename", projectId }),
          ),
      },
      {
        key: "tab-stops",
        subscribe: () => limitTabStops(document.body),
      },
      {
        key: "composer-focus",
        subscribe: () => listenForComposerFocus(document),
      },
      {
        key: "composer-size",
        // Typing and resizing the window both change how many lines the text needs.
        subscribe: () => {
          const fit = (event: Event) => {
            const target = event.target;
            if (
              target instanceof HTMLTextAreaElement &&
              target.id === "composer"
            )
              fitComposer(target);
          };
          const refit = () => {
            const composer =
              document.querySelector<HTMLTextAreaElement>("#composer");
            if (composer) fitComposer(composer);
          };
          document.addEventListener("input", fit);
          window.addEventListener("resize", refit);
          return () => {
            document.removeEventListener("input", fit);
            window.removeEventListener("resize", refit);
          };
        },
      },
      {
        key: "fullscreen",
        subscribe: (dispatch) => {
          const changed = () =>
            dispatch({
              type: "fullscreenChanged",
              active: !!document.fullscreenElement,
            });
          document.addEventListener("fullscreenchange", changed);
          return () =>
            document.removeEventListener("fullscreenchange", changed);
        },
      },
    ],
  },
  runEffect: run,
});
