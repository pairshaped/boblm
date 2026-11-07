import { usageMeters, usageResetSubscription } from "./usage.ts";
import type { SubagentConversation } from "./subagent-conversation.ts";
import {
  emptyTimeline,
  projectEvents,
  mergeTimeline,
  type Timeline,
} from "./timeline.ts";
import { groupToolRows, type VisibleRow } from "./tool-groups.ts";
import { renderMarkdown } from "./markdown.ts";
import { messageTime } from "./message-time.ts";
import {
  subagentsFrom,
  subagentsFromEvents,
  subagentsFromSessions,
  type Subagent,
} from "./subagents.ts";
import { questionsContent } from "./questions.ts";
import { speechText, supportsSpeech } from "./speech.ts";
import {
  h,
  bindEvents,
  defineProgram,
  emptySubscription,
  type ProgramSubscription,
  type VNode,
} from "@pairshaped/hypertea/program";
import {
  type SessionInfo,
  type QuestionInfo,
  type QuestionsReply,
  type ModelOption,
  type SessionStatus,
  type StoredEvent,
  type AttachmentInfo,
  type SessionListEvent,
  type ProjectInfo,
  type DirectoryListing,
  type ChatHistoryPage,
  type SubagentHistoryPage,
  contractSchema,
  parseStoredEvent,
  parseSessionListEvent,
  parseSequence,
  limits,
} from "./generated/contract.ts";
import { chatStream } from "./stream.ts";
import type { LiveHealth } from "@pairshaped/hypertea/live";
import type { CommandOutcome } from "./http.ts";
const MAX_ATTACHMENTS = limits.maxAttachments;
const MAX_UPLOAD_BYTES = limits.maxUploadBytes;
export type DraftAttachment = {
  id: string;
  name: string;
  contentType: string;
  size: number;
  state: "uploading" | "ready" | "failed";
  error?: string;
};
export type AttachItem = { id: string; file: File };
type Sent = {
  id: string;
  content: string;
  attachments: ReadonlyArray<string>;
  preserveDraft?: boolean;
};
// Only the types the server serves inline can be shown as thumbnails.
const isImage = (contentType: string) =>
  ["image/png", "image/jpeg", "image/gif", "image/webp"].includes(contentType);
export function sizeText(size: number): string {
  if (size >= 1 << 20) return `${(size / (1 << 20)).toFixed(1)} MiB`;
  if (size >= 1 << 10) return `${Math.round(size / (1 << 10))} KiB`;
  return `${size} B`;
}
function thumbnailIds(model: Model): Set<string> {
  return new Set([
    ...Object.values(model.attachments).flatMap((items) =>
      items.map((a) => a.id),
    ),
    ...Object.values(model.history).flatMap((h) =>
      h.timeline.rows.flatMap((r) => r.attachments?.map((a) => a.id) ?? []),
    ),
  ]);
}
function pruneThumbnails(
  model: Model,
  effects: ReadonlyArray<Effect>,
): readonly [Model, ReadonlyArray<Effect>] {
  const keep = thumbnailIds(model);
  const discarded = Object.entries(model.thumbs).filter(
    ([id]) => !keep.has(id),
  );
  if (!discarded.length) return [model, effects];
  return [
    {
      ...model,
      thumbs: Object.fromEntries(
        Object.entries(model.thumbs).filter(([id]) => keep.has(id)),
      ),
    },
    [
      ...effects,
      ...discarded.map(([id, url]) => ({ type: "discard" as const, id, url })),
    ],
  ];
}
function sendBody(sessionId: string, sent: Sent) {
  return {
    sessionId,
    messageId: sent.id,
    content: sent.content,
    ...(sent.attachments.length ? { attachments: sent.attachments } : {}),
  };
}
/** The model and effort a new chat starts with, chosen in the New chat dialog. */
export type Choice = { model: string; effort: string | null };
export type DragItem =
  | { kind: "project"; id: string }
  | { kind: "chat"; id: string; projectId: string };
export type HistoryDirection = "latest" | "older" | "newer";
export type HistoryState = {
  timeline: Timeline;
  loaded: boolean;
  loading: boolean;
  hasOlder: boolean;
  hasNewer: boolean;
  following: boolean;
  generation: number;
  direction: HistoryDirection;
  error: string;
};
const emptyHistory = (): HistoryState => ({
  timeline: emptyTimeline(),
  loaded: false,
  loading: false,
  hasOlder: false,
  hasNewer: false,
  following: true,
  generation: 0,
  direction: "latest",
  error: "",
});
export type Model = {
  fullscreen: boolean;
  directoryPicker: {
    listing: DirectoryListing | null;
    loading: boolean;
    error: string;
  } | null;
  directoryRequest: number;
  dragging: DragItem | null;
  dropTarget: { id: string; after: boolean } | null;
  /** Serialize saves; a failed save gets one authoritative list refresh. */
  reorderState: "saving" | "refreshing" | null;
  /** Reject list snapshots requested before a reorder began or settled. */
  orderGeneration: number;
  token: string;
  connectionPending: boolean;
  connectionGeneration: number;
  sessions: ReadonlyArray<SessionInfo>;
  models: ReadonlyArray<ModelOption>;
  projects: ReadonlyArray<ProjectInfo>;
  /** The open New chat dialog: which project it is for and what is picked so far. */
  newChat: ({ projectId: string; name: string } & Choice) | null;
  renameChat: {
    sessionId: string;
    name: string;
    saving: boolean;
    error: string;
  } | null;
  /** What the last chat was created with, offered first next time. */
  lastChoice: Choice | null;
  /** Projects folded shut in the sidebar; their chats show no number. */
  collapsed: Readonly<Record<string, true>>;
  /** Uncertain messages the owner hid from the held-queue banner, by message UUID. */
  dismissed: Readonly<Record<string, true>>;
  /** Explicit folding choices on this page, scoped by chat and row key; peers start folded. */
  collapsedMessages: Readonly<Record<string, boolean>>;
  /** Whether the shared questions and agents drawer is open. */
  drawerOpen: boolean;
  /** On a phone, whether the chat list covers the screen. It also shows with no chat open or no login. */
  sidebarOpen: boolean;
  /** The chat that was open when the page loaded, reopened once the chat list arrives. */
  restoreChat: string | null;
  /** The message being read aloud, by row key. */
  speaking: string | null;
  /** The image open full screen, with the label shown over it. */
  pushState: "unavailable" | "available" | "enabled";
  drawerTab: "agents" | "jobs" | "questions";
  subagentConversation: SubagentConversation | null;
  subagentGeneration: number;
  subagents: ReadonlyArray<Subagent>;
  questions: ReadonlyArray<QuestionInfo>;
  questionGeneration: number;
  questionDrafts: Readonly<Record<string, string>>;
  questionChoices: Readonly<Record<string, number | "discuss_later" | null>>;
  questionPendingCount: number;
  questionSending: Readonly<Record<string, true>>;
  questionError: string;
  questionToFocus: string | null;
  lightbox: { id: string; label: string; returnId?: string } | null;
  /** The message whose reading failed, shown beside its button until the next reading. */
  speechFailed: string | null;
  active: string | null;
  /** Chats the server has deleted. Their late replies are ignored instead of shown as errors. */
  forgotten: Readonly<Record<string, true>>;
  events: Readonly<Record<string, ReadonlyArray<StoredEvent>>>;
  cursors: Readonly<Record<string, number>>;
  history: Readonly<Record<string, HistoryState>>;
  historyGeneration: number;
  /** Time the document became hidden; null while visible. */
  hiddenSince: number | null;
  listCursor: number;
  listGeneration: number;
  /** Count of list requests sent; each carries its number so a late reply can be told apart. */
  listRequests: number;
  /** The one list request awaiting its reply, or null. */
  listInFlight: number | null;
  /** A refetch was wanted while one was in flight; one follow-up covers all of them. */
  listQueued: boolean;
  drafts: Readonly<Record<string, string>>;
  pending: Readonly<Record<string, Sent>>;
  uncertain: Readonly<Record<string, Sent>>;
  /** Why the last send in a session failed, shown beside the composer with a Retry. */
  sendError: Readonly<Record<string, string>>;
  attachments: Readonly<Record<string, ReadonlyArray<DraftAttachment>>>;
  thumbs: Readonly<Record<string, string>>;
  error: string;
  newMessages: boolean;
  /** Why each live stream is interrupted right now, by stream; empty when all is well. */
  connection: Readonly<Record<string, string>>;
};
export type Msg =
  | { type: "promptProjectRename"; projectId: string }
  | { type: "renameProject"; projectId: string; name: string }
  | { type: "toggleFullscreen" }
  | { type: "fullscreenChanged"; active: boolean }
  | { type: "usageExpired" }
  | { type: "visibility"; hidden: boolean; now: number; idle?: boolean }
  | { type: "history"; direction: HistoryDirection; sessionId?: string }
  | { type: "following"; value: boolean }
  | {
      type: "historyReply";
      sessionId: string;
      generation: number;
      outcome: CommandOutcome<ChatHistoryPage>;
    }
  | { type: "openDirectories" }
  | { type: "browseDirectory"; path: string }
  | { type: "closeDirectories" }
  | { type: "chooseDirectory"; id: string }
  | { type: "dragStart"; item: DragItem }
  | { type: "dragOver"; target: DragItem; after: boolean }
  | { type: "dragLeave"; id: string }
  | { type: "dragEnd" }
  | { type: "drop"; target: DragItem; after: boolean }
  | { type: "login"; token: string }
  | {
      type: "connectionReply";
      generation: number;
      outcome: CommandOutcome<ChatList>;
    }
  | { type: "logout" }
  | { type: "requestLock" }
  | {
      type: "list";
      sessions: ReadonlyArray<SessionInfo>;
      models: ReadonlyArray<ModelOption>;
      projects: ReadonlyArray<ProjectInfo>;
    }
  | { type: "select"; id: string }
  | { type: "draft"; text: string }
  | { type: "compact"; sessionId: string; id: string }
  | { type: "send"; id: string }
  | { type: "retrySend" }
  | { type: "discardHeld" }
  | { type: "attach"; items: ReadonlyArray<AttachItem> }
  | {
      type: "uploaded";
      sessionId: string;
      id: string;
      outcome: CommandOutcome<unknown>;
    }
  | { type: "retryUpload"; id: string }
  | { type: "removeAttachment"; id: string }
  | { type: "thumb"; id: string; url: string }
  | { type: "download"; id: string; name: string }
  | { type: "removeProject"; projectId: string }
  | { type: "requestRemoveProject"; projectId: string }
  | { type: "openNewChat"; projectId: string }
  | { type: "closeNewChat" }
  | { type: "submitSecret"; requestId: string; value: string }
  | { type: "declineSecret"; requestId: string }
  | { type: "toggleProject"; projectId: string }
  | { type: "toggleDrawer" }
  | { type: "toggleSidebar" }
  | {
      type: "pushState";
      state: "unavailable" | "available" | "enabled";
      error?: string;
    }
  | { type: "reloadPage"; sessionId: string }
  | { type: "togglePush" }
  | { type: "selectDrawerTab"; tab: "agents" | "jobs" | "questions" }
  | { type: "openSubagent"; key: string }
  | { type: "backToAgents" }
  | { type: "subagentFollowing"; value: boolean }
  | { type: "subagentPage"; latest: boolean }
  | {
      type: "subagentReply";
      generation: number;
      owner: string;
      key: string;
      outcome: CommandOutcome<SubagentHistoryPage>;
    }
  | { type: "questionDraft"; id: string; text: string }
  | {
      type: "questionChoice";
      id: string;
      choice: number | "discuss_later" | null;
    }
  | { type: "discussQuestionLater"; id: string }
  | { type: "answerQuestion"; id: string; answer: string }
  | { type: "reloadQuestions" }
  | { type: "openImage"; id: string }
  | { type: "closeImage" }
  | { type: "stepImage"; direction: -1 | 1 }
  | { type: "backdropClick"; onImage: boolean }
  | { type: "readAloud"; key: string; text: string }
  | { type: "stopReading" }
  | { type: "speechEnded"; key: string; failed: boolean }
  | { type: "toggleMessage"; key: string; collapsed?: boolean }
  | { type: "copyText"; text: string; source?: HTMLElement }
  | { type: "dismissUncertain"; uuid: string }
  | { type: "sendAgain"; uuid: string; text: string }
  | { type: "jump"; number: number }
  | { type: "refresh" }
  | { type: "newChatName"; name: string }
  | { type: "requestDelete" }
  | { type: "deleteChat"; sessionId: string }
  | { type: "requestDeleteChat"; sessionId: string }
  | { type: "openRename" }
  | { type: "renameName"; name: string }
  | { type: "closeRename" }
  | { type: "saveRename" }
  | { type: "newChatModel"; model: string }
  | { type: "newChatEffort"; effort: string | null }
  | { type: "createChat"; id: string }
  | { type: "command"; name: string }
  | { type: "commandOn"; name: string; sessionId: string }
  | { type: "choice"; model?: string; effort?: string | null }
  | { type: "event"; event: StoredEvent }
  | { type: "unknownEvent"; sessionId: string; seq: number }
  | { type: "unknownListEvent"; seq: number }
  | { type: "listEvent"; event: SessionListEvent }
  | {
      type: "reply";
      orderGeneration?: number;
      listRequest?: number;
      name: string;
      sessionId: string;
      outcome: CommandOutcome<unknown>;
    }
  | { type: "retryMessage"; uuid: string }
  | { type: "error"; message: string }
  | { type: "newMessages"; visible: boolean }
  | { type: "bottom" }
  | { type: "connection"; stream: string; health: LiveHealth }
  | { type: "noop" };
export type Effect =
  | { type: "connect"; token: string; generation: number }
  | { type: "promptProjectRename"; projectId: string; name: string }
  | { type: "toggleFullscreen" }
  | {
      type: "historyRequest";
      sessionId: string;
      generation: number;
      direction: HistoryDirection;
      position?: number;
    }
  | {
      type: "historyScroll";
      sessionId: string;
      generation: number;
      latest: boolean;
      target?: "chat" | "subagent";
    }
  | {
      type: "request";
      name: string;
      body: unknown;
      sessionId: string;
      orderGeneration?: number;
      listRequest?: number;
    }
  | { type: "persist"; token: string; drafts: Readonly<Record<string, string>> }
  | { type: "composer"; sessionId: string; text: string }
  | { type: "persistChoice"; choice: Choice }
  | { type: "persistCollapsed"; collapsed: Readonly<Record<string, true>> }
  | { type: "copy"; text: string; source?: HTMLElement }
  | { type: "speak"; key: string; text: string }
  | { type: "stopSpeech" }
  | { type: "reloadPage"; model: Model }
  | { type: "togglePush"; enabled: boolean; token: string }
  | { type: "focusQuestion"; id: string }
  | { type: "focusLightbox" }
  | { type: "focusImage"; id: string }
  | { type: "persistLastChat"; id: string | null }
  | { type: "persistList"; list: ChatList }
  | { type: "persistDismissed"; dismissed: Readonly<Record<string, true>> }
  | { type: "confirmDelete"; sessionId: string; name: string }
  | {
      type: "confirmRemoveProject";
      projectId: string;
      name: string;
      chats: number;
    }
  | { type: "confirmLock" }
  | { type: "directoryFocus"; open: boolean }
  | { type: "persistUncertain"; uncertain: Readonly<Record<string, Sent>> }
  | {
      type: "persistAttachments";
      attachments: Readonly<Record<string, ReadonlyArray<DraftAttachment>>>;
    }
  | { type: "upload"; sessionId: string; id: string; file?: File }
  | { type: "discard"; id: string; url?: string }
  | { type: "thumbnail"; sessionId: string; id: string; file?: File }
  | { type: "download"; sessionId: string; id: string; name: string }
  | { type: "bottom" };
const on = bindEvents<Msg>();
const request = (
  name: string,
  body: unknown,
  sessionId = "",
  orderGeneration?: number,
): Effect => ({
  type: "request",
  name,
  body,
  sessionId,
  ...(orderGeneration === undefined ? {} : { orderGeneration }),
});
/** A model's short label from the catalog; a model the catalog dropped shows its own ID. */
function modelLabel(models: ReadonlyArray<ModelOption>, id: string): string {
  return models.find((option) => option.model === id)?.label ?? id;
}
/** The chats on screen: the viewed chat and, in its drawer, the open boblm subagent. */
function openChats(model: Model): string[] {
  const view = model.subagentConversation;
  return [
    ...(model.active ? [model.active] : []),
    ...(view?.boblm === true && view.owner === model.active ? [view.key] : []),
  ];
}
/**
 * Per-chat state keeps only the chats on screen and the one chat being written. A write to
 * one of them must not drop the other, so the map is rebuilt from the open chats every time.
 */
function keepViewed<T>(
  model: Model,
  source: Readonly<Record<string, T>>,
  id: string,
  value: T,
): Record<string, T> {
  const kept: Record<string, T> = {};
  for (const open of openChats(model))
    if (open !== id && open in source) kept[open] = source[open]!;
  kept[id] = value;
  return kept;
}
function listRequest(model: Model): Extract<Effect, { type: "request" }> {
  return {
    type: "request",
    name: "listSessions",
    body: {},
    sessionId: "",
    ...(model.orderGeneration
      ? { orderGeneration: model.orderGeneration }
      : {}),
  };
}
/** What the chat list holds: the sessions, their projects, and the models on offer. */
export type ChatList = {
  sessions: ReadonlyArray<SessionInfo>;
  projects: ReadonlyArray<ProjectInfo>;
  models: ReadonlyArray<ModelOption>;
};
export function initial(
  token = "",
  drafts: Readonly<Record<string, string>> = {},
  attachments: Readonly<Record<string, ReadonlyArray<DraftAttachment>>> = {},
  uncertain: Readonly<Record<string, Sent>> = {},
  lastChoice: Choice | null = null,
  collapsed: Readonly<Record<string, true>> = {},
  dismissed: Readonly<Record<string, true>> = {},
  drawerOpen = false,
  restoreChat: string | null = null,
  cachedList: ChatList | null = null,
): Model {
  return {
    token,
    connectionPending: false,
    connectionGeneration: 0,
    fullscreen: false,
    directoryPicker: null,
    directoryRequest: 0,
    dragging: null,
    dropTarget: null,
    reorderState: null,
    orderGeneration: 0,
    sessions: cachedList?.sessions ?? [],
    models: cachedList?.models ?? [],
    projects: cachedList?.projects ?? [],
    newChat: null,
    renameChat: null,
    lastChoice,
    collapsed,
    dismissed,
    collapsedMessages: {},
    drawerOpen,
    sidebarOpen: false,
    restoreChat,
    speaking: null,
    pushState: "unavailable",
    drawerTab: "agents",
    subagentConversation: null,
    subagentGeneration: 0,
    subagents: [],
    questions: [],
    questionGeneration: 0,
    questionDrafts: {},
    questionChoices: {},
    questionPendingCount: 0,
    questionSending: {},
    questionError: "",
    questionToFocus: null,
    lightbox: null,
    speechFailed: null,
    active: null,
    forgotten: {},
    events: {},
    cursors: {},
    history: {},
    historyGeneration: 0,
    hiddenSince: null,
    listCursor: 0,
    listGeneration: 0,
    listRequests: 0,
    listInFlight: null,
    listQueued: false,
    drafts,
    pending: {},
    uncertain,
    // A send held before a reload keeps its UUID; say so and offer the retry.
    sendError: Object.fromEntries(
      Object.keys(uncertain).map((id) => [
        id,
        "Earlier send outcome unknown. Retry to confirm; it keeps its original ID.",
      ]),
    ),
    attachments,
    thumbs: {},
    error: "",
    newMessages: false,
    connection: {},
  };
}
/** Uploaded files survive a reload with the draft; unfinished uploads cannot. */
function readyOnly(model: Model) {
  return Object.fromEntries(
    Object.entries(model.attachments)
      .map(([id, list]) => [id, list.filter((a) => a.state === "ready")])
      .filter(([, list]) => (list as DraftAttachment[]).length > 0),
  ) as Model["attachments"];
}
function persistAttachments(model: Model): Effect {
  return { type: "persistAttachments", attachments: readyOnly(model) };
}
function without<T>(
  record: Readonly<Record<string, T>>,
  key: string,
): Readonly<Record<string, T>> {
  const rest = { ...record };
  delete rest[key];
  return rest;
}
/** Sends whose outcome is not known: unconfirmed ones and ones still in flight. */
function heldSends(model: Model): Readonly<Record<string, Sent>> {
  return { ...model.uncertain, ...model.pending };
}
function persistUncertain(model: Model): Effect {
  return { type: "persistUncertain", uncertain: heldSends(model) };
}
function setAttachments(
  model: Model,
  sessionId: string,
  list: ReadonlyArray<DraftAttachment>,
): Model {
  return { ...model, attachments: { ...model.attachments, [sessionId]: list } };
}
export type SecretRequest = {
  requestId: string;
  file: string;
  key: ReadonlyArray<string>;
  expiresAt: number;
};
/** Credential requests the owner has not answered yet: asked, and not yet resolved. */
export function pendingSecrets(
  events: ReadonlyArray<StoredEvent>,
): ReadonlyArray<SecretRequest> {
  const open = new Map<string, SecretRequest>();
  for (const e of events) {
    if (e.kind === "secret_requested")
      open.set(e.payload.request_id, {
        requestId: e.payload.request_id,
        file: e.payload.file,
        key: e.payload.key,
        expiresAt: e.payload.expires_at,
      });
    else if (e.kind === "secret_resolved") open.delete(e.payload.request_id);
  }
  return [...open.values()];
}
/** A role subagent is hidden from the sidebar; roots have no parent or root link. */
function rootChat(session: SessionInfo): boolean {
  return session.parentSessionId == null && session.rootSessionId == null;
}
/** The chats showing in the sidebar, in order: project by project, skipping folded ones. */
/** The numbered chats: active ones in open projects. An archived chat has no number or shortcut. */
export function visibleChats(model: Model): ReadonlyArray<SessionInfo> {
  return model.projects
    .filter((p) => !model.collapsed[p.projectId])
    .flatMap((p) =>
      model.sessions.filter(
        (s) => s.projectId === p.projectId && rootChat(s) && !s.status.archived,
      ),
    );
}
/** Command plus a digit (Alt plus a digit off a Mac) names the chat with that number. */
export function shortcutTarget(key: {
  code: string;
  metaKey?: boolean;
  altKey?: boolean;
  ctrlKey?: boolean;
  shiftKey?: boolean;
  repeat?: boolean;
}): number | null {
  if (key.repeat || key.ctrlKey || key.shiftKey) return null;
  if (!key.metaKey && !key.altKey) return null;
  const match = /^Digit([1-9])$/.exec(key.code);
  return match ? Number(match[1]) : null;
}
function status(model: Model, id: string, value: SessionStatus): Model {
  return {
    ...model,
    sessions: model.sessions.map((s) =>
      s.sessionId === id ? { ...s, status: value } : s,
    ),
  };
}
function dragItemExists(model: Model, item: DragItem): boolean {
  return item.kind === "project"
    ? model.projects.some((p) => p.projectId === item.id)
    : model.sessions.some(
        (s) => s.sessionId === item.id && s.projectId === item.projectId,
      );
}
function acceptsDrop(model: Model, target: DragItem): boolean {
  const source = model.dragging;
  return (
    !model.reorderState &&
    !!source &&
    source.id !== target.id &&
    source.kind === target.kind &&
    dragItemExists(model, source) &&
    dragItemExists(model, target) &&
    (source.kind === "project" ||
      (target.kind === "chat" && source.projectId === target.projectId))
  );
}
/** Move within a list. Returning the same array means the requested slot is unchanged. */
function moved<T>(
  items: ReadonlyArray<T>,
  from: number,
  target: number,
  after: boolean,
): ReadonlyArray<T> {
  const at = target - (from < target ? 1 : 0) + (after ? 1 : 0);
  if (from === target || from === at) return items;
  const result = [...items];
  result.splice(from, 1);
  result.splice(at, 0, items[from]!);
  return result;
}
/** Send one list request and remember it, so its reply can be matched and a late one refused. */
function sendList(model: Model): [Model, Effect] {
  const id = model.listRequests + 1;
  return [
    { ...model, listRequests: id, listInFlight: id, listQueued: false },
    { ...listRequest(model), listRequest: id },
  ];
}
const isListRequest = (effect: Effect) =>
  effect.type === "request" && effect.name === "listSessions";
/**
 * Every message passes through here so the list is never fetched in a storm. One request is in
 * flight at a time; any number of wants meanwhile collapse into one follow-up sent when it
 * settles. A reply to a request that is no longer the current one is ignored whole, so an older
 * list cannot overwrite a newer one or move the list stream's cursor backwards.
 */
export function update(
  model: Model,
  msg: Msg,
): readonly [Model, ReadonlyArray<Effect>] {
  let current = model;
  if (
    msg.type === "reply" &&
    msg.name === "listSessions" &&
    msg.listRequest !== undefined
  ) {
    if (msg.listRequest !== model.listInFlight) return [model, []];
    current = { ...model, listInFlight: null };
  } else if (msg.type === "refresh") {
    // A tab returning from the background asks again at once; any earlier request is abandoned.
    current = { ...model, listInFlight: null, listQueued: false };
  }
  const [stepped, produced] = step(current, msg);
  const effects = produced.filter((effect) => !isListRequest(effect));
  const wanted =
    effects.length !== produced.length ||
    (stepped.listInFlight === null && stepped.listQueued);
  if (!wanted) return [stepped, effects];
  if (stepped.listInFlight !== null)
    return [{ ...stepped, listQueued: true }, effects];
  const [sent, request] = sendList(stepped);
  return [sent, [...effects, request]];
}
function step(model: Model, msg: Msg): readonly [Model, ReadonlyArray<Effect>] {
  switch (msg.type) {
    case "usageExpired":
      return [{ ...model }, []];
    case "noop":
      return [model, []];
    case "connection":
      return [
        {
          ...model,
          connection:
            msg.health.kind === "healthy"
              ? without(model.connection, msg.stream)
              : { ...model.connection, [msg.stream]: msg.health.text },
        },
        [],
      ];
    case "login": {
      if (model.connectionPending) return [model, []];
      const token = msg.token.trim();
      if (!token)
        return [{ ...model, error: "Enter your server token to connect." }, []];
      const generation = model.connectionGeneration + 1;
      return [
        {
          ...model,
          token,
          connectionPending: true,
          connectionGeneration: generation,
          error: "",
        },
        [{ type: "connect", token, generation }],
      ];
    }
    case "connectionReply": {
      if (
        !model.connectionPending ||
        msg.generation !== model.connectionGeneration
      )
        return [model, []];
      if (msg.outcome.kind !== "replied") {
        const cause =
          msg.outcome.kind === "rejected"
            ? msg.outcome.error.message
            : (msg.outcome.reason ?? "No reply arrived.");
        return [
          {
            ...model,
            token: "",
            connectionPending: false,
            error: `Could not connect: ${cause} Check your token and connection, then try again.`,
          },
          [],
        ];
      }
      const [connected, effects] = step(
        { ...model, connectionPending: false },
        {
          type: "reply",
          name: "listSessions",
          sessionId: "",
          orderGeneration: model.orderGeneration,
          outcome: msg.outcome,
        },
      );
      return [
        connected,
        [
          { type: "persist", token: model.token, drafts: model.drafts },
          ...effects,
        ],
      ];
    }
    case "toggleFullscreen":
      return [model, [{ type: "toggleFullscreen" }]];
    case "fullscreenChanged":
      return [{ ...model, fullscreen: msg.active }, []];
    case "requestLock":
      return model.token ? [model, [{ type: "confirmLock" }]] : [model, []];
    case "logout":
      return [
        // Locking disconnects; it must not throw away the draft or its finished uploads.
        {
          ...initial(
            "",
            model.drafts,
            readyOnly(model),
            heldSends(model),
            model.lastChoice,
            model.collapsed,
            model.dismissed,
            model.drawerOpen,
            // Unlocking returns to the chat that was open.
            model.active ?? model.restoreChat,
          ),
          fullscreen: model.fullscreen,
          thumbs: model.thumbs,
          directoryRequest: model.directoryRequest + 1,
          connectionGeneration: model.connectionGeneration + 1,
          historyGeneration: model.historyGeneration + 1,
          orderGeneration: model.orderGeneration + 1,
        },
        [{ type: "persist", token: "", drafts: model.drafts }],
      ];
    case "list":
      if (model.reorderState === "saving") return [model, []];
      return [
        {
          ...model,
          sessions: msg.sessions,
          models: msg.models,
          projects: msg.projects,
        },
        [],
      ];
    case "select": {
      const effects: Effect[] = [
        {
          type: "composer",
          sessionId: msg.id,
          text: model.drafts[msg.id] ?? "",
        },
      ];
      effects.push({ type: "persistLastChat", id: msg.id });
      // Picking a chat closes the phone's chat list.
      if (model.active === msg.id)
        return [{ ...model, sidebarOpen: false }, effects];
      // Another chat's message is no longer on screen, so its reading stops.
      if (model.speaking) effects.push({ type: "stopSpeech" });
      const selected = {
        ...model,
        speaking: null,
        lightbox: null,
        subagentConversation: null,
        subagents: [],
        questions: [],
        questionPendingCount: 0,
        questionToFocus: model.active === null ? model.questionToFocus : null,
        questionError: "",
        questionSending: {},
        questionGeneration: model.questionGeneration + 1,
        sidebarOpen: false,
        active: msg.id,
        newMessages: false,
        error: "",
        connection: without(model.connection, "this chat"),
        history: {},
        events: {},
        cursors: {},
      };
      const [next, load] = step(selected, {
        type: "history",
        direction: "latest",
      });
      return pruneThumbnails(next, [
        ...effects,
        request(
          "listQuestions",
          { sessionId: msg.id },
          msg.id,
          next.questionGeneration,
        ),
        ...load,
      ]);
    }
    case "visibility": {
      if (msg.hidden)
        return [{ ...model, hiddenSince: model.hiddenSince ?? msg.now }, []];
      const away = model.hiddenSince === null ? 0 : msg.now - model.hiddenSince;
      const visible = { ...model, hiddenSince: null, listGeneration: 0 };
      const [refreshed, effects] = step(visible, { type: "refresh" });
      if (
        (!msg.idle && away < limits.historyResumeLatestMs) ||
        !refreshed.active ||
        !refreshed.token ||
        refreshed.connectionPending
      )
        return [refreshed, effects];
      const id = refreshed.active;
      // Reuse chat selection so old rows, event buffers and cursors are released together.
      const [next, load] = step(
        { ...refreshed, active: null },
        { type: "select", id },
      );
      return [
        { ...next, subagents: refreshed.subagents },
        [...effects, ...load],
      ];
    }
    case "following": {
      const id = model.active;
      const current = id ? model.history[id] : undefined;
      return id && current && current.following !== msg.value
        ? [
            {
              ...model,
              history: {
                ...model.history,
                [id]: { ...current, following: msg.value },
              },
            },
            [],
          ]
        : [model, []];
    }
    case "history": {
      const id = msg.sessionId ?? model.active;
      if (!id) return [model, []];
      const subagent =
        msg.sessionId !== undefined &&
        model.subagentConversation?.boblm === true &&
        model.subagentConversation.owner === model.active &&
        model.subagentConversation.key === id;
      if (msg.sessionId !== undefined && !subagent) return [model, []];
      const current = model.history[id] ?? emptyHistory();
      if (current.loading) return [model, []];
      if (
        (msg.direction === "older" && !current.hasOlder) ||
        (msg.direction === "newer" && !current.hasNewer)
      )
        return [model, []];
      const generation = model.historyGeneration + 1;
      const position =
        msg.direction === "older"
          ? current.timeline.rows[0]?.seq
          : msg.direction === "newer"
            ? current.timeline.rows.at(-1)?.seq
            : undefined;
      return [
        {
          ...model,
          historyGeneration: generation,
          history: keepViewed(model, model.history, id, {
            ...current,
            loading: true,
            generation,
            direction: msg.direction,
            error: "",
          }),
        },
        [
          {
            type: "historyRequest",
            sessionId: id,
            generation,
            direction: msg.direction,
            ...(position === undefined ? {} : { position }),
          },
        ],
      ];
    }
    case "historyReply": {
      const id = msg.sessionId;
      if (model.forgotten[id]) return [model, []];
      const subagent =
        model.subagentConversation?.boblm === true &&
        model.subagentConversation.owner === model.active &&
        model.subagentConversation.key === id;
      const current = model.history[id];
      if (
        (id !== model.active && !subagent) ||
        !current ||
        current.generation !== msg.generation
      )
        return [model, []];
      if (msg.outcome.kind !== "replied") {
        const reason =
          msg.outcome.kind === "rejected"
            ? msg.outcome.error.message
            : (msg.outcome.reason ?? "the connection closed");
        return [
          {
            ...model,
            history: keepViewed(model, model.history, id, {
              ...current,
              loading: false,
              error: `Chat history could not be loaded: ${reason}. Try loading it again.`,
            }),
          },
          [
            {
              type: "historyScroll",
              sessionId: id,
              generation: msg.generation,
              latest: false,
              ...(subagent ? { target: "subagent" as const } : {}),
            },
          ],
        ];
      }
      const page = msg.outcome.reply;
      if (
        page.sessionId !== id ||
        page.events.some((e) => e.sessionId !== id) ||
        page.context.some((e) => e.sessionId !== id)
      )
        return step(model, {
          ...msg,
          outcome: {
            kind: "uncertain",
            reason: "the history reply belonged to another chat",
          },
        });
      const agent = agentName(
        model.sessions.find((s) => s.sessionId === id)?.agent ?? "claude",
      );
      const projected = projectEvents(
        emptyTimeline(),
        page.events,
        agent,
        page.cursor,
      );
      const incoming = {
        ...projected,
        partial: page.partialId ?? null,
        partialAt: page.cursor,
      };
      let timeline =
        current.direction === "latest"
          ? mergeTimeline(incoming, {
              ...current.timeline,
              rows: current.timeline.rows.filter(
                (row) => row.revision > page.cursor,
              ),
            })
          : mergeTimeline(current.timeline, incoming);
      if (
        current.direction === "latest" &&
        current.timeline.partialAt > page.cursor
      )
        timeline = {
          ...timeline,
          partial: current.timeline.partial,
          partialAt: current.timeline.partialAt,
        };
      let hasOlder =
        current.direction === "newer" ? current.hasOlder : page.hasMore;
      let hasNewer =
        current.direction === "newer"
          ? page.hasMore
          : current.direction === "latest"
            ? false
            : current.hasNewer;
      if (timeline.rows.length > limits.historyMaxRows) {
        if (current.direction === "older") {
          timeline = {
            ...timeline,
            rows: timeline.rows.slice(0, limits.historyMaxRows),
          };
          hasNewer = true;
        } else {
          timeline = {
            ...timeline,
            rows: timeline.rows.slice(-limits.historyMaxRows),
          };
          hasOlder = true;
        }
      }
      const effects: Effect[] = [
        {
          type: "historyScroll",
          sessionId: id,
          generation: msg.generation,
          latest: current.direction === "latest",
          ...(subagent ? { target: "subagent" as const } : {}),
        },
      ];
      // Read questions after the snapshot cursor too: a change between the first
      // question read and this history snapshot would otherwise be behind the SSE cursor.
      const questionGeneration =
        current.direction === "latest" && !subagent
          ? model.questionGeneration + 1
          : model.questionGeneration;
      if (current.direction === "latest" && !subagent)
        effects.push(
          request("listQuestions", { sessionId: id }, id, questionGeneration),
        );
      for (const row of incoming.rows)
        for (const a of row.attachments ?? [])
          if (isImage(a.contentType) && !model.thumbs[a.id])
            effects.push({ type: "thumbnail", sessionId: id, id: a.id });
      return pruneThumbnails(
        {
          ...status(model, id, page.status),
          questionGeneration,
          subagents: subagent
            ? model.subagents
            : subagentsFromEvents(
                [...page.context, ...page.events].sort((a, b) => a.seq - b.seq),
                model.subagents,
              ),
          events: keepViewed(
            model,
            model.events,
            id,
            page.cursor >= (model.cursors[id] ?? 0)
              ? page.context
              : (model.events[id] ?? []),
          ),
          cursors: {
            ...model.cursors,
            [id]:
              current.direction === "latest"
                ? Math.max(page.cursor, model.cursors[id] ?? 0)
                : (model.cursors[id] ?? page.cursor),
          },
          newMessages: subagent ? model.newMessages : hasNewer,
          history: keepViewed(model, model.history, id, {
            ...current,
            timeline,
            loaded: true,
            loading: false,
            hasOlder,
            hasNewer,
            following: current.direction === "latest",
            error: "",
          }),
        },
        effects,
      );
    }
    case "draft": {
      if (!model.active) return [model, []];
      const drafts = { ...model.drafts, [model.active]: msg.text };
      return [
        { ...model, drafts },
        [{ type: "persist", token: model.token, drafts }],
      ];
    }
    case "compact": {
      const id = msg.sessionId;
      const session = model.sessions.find((s) => s.sessionId === id);
      if (!session || session.status.archived) return [model, []];
      if (model.pending[id] || model.uncertain[id])
        return [
          {
            ...model,
            error:
              "Compaction was not sent because another send is pending or unconfirmed. Resolve that send, then try again.",
          },
          [],
        ];
      const pending: Sent = {
        id: msg.id,
        content: "/compact",
        attachments: [],
        preserveDraft: true,
      };
      const next = {
        ...model,
        pending: { ...model.pending, [id]: pending },
        sendError: without(model.sendError, id),
        error: "",
      };
      return [
        next,
        [
          persistUncertain(next),
          request("sendMessage", sendBody(id, pending), id),
        ],
      ];
    }
    case "send": {
      const id = model.active;
      if (!id || model.pending[id]) return [model, []];
      const drafts = model.attachments[id] ?? [];
      const content = model.drafts[id] ?? "";
      const ready = drafts.filter((a) => a.state === "ready");
      if (!content.trim() && ready.length === 0) return [model, []];
      const sentIds = ready.map((a) => a.id);
      if (model.uncertain[id])
        return [
          {
            ...model,
            sendError: {
              ...model.sendError,
              [id]: "An earlier send is unconfirmed, and it may still arrive. Retry it, or discard it, before sending anything else.",
            },
          },
          [],
        ];
      if (drafts.some((a) => a.state !== "ready"))
        return [
          {
            ...model,
            error: "Wait for uploads to finish, or remove failed ones.",
          },
          [],
        ];
      const pending = { id: msg.id, content, attachments: sentIds };
      const next = {
        ...model,
        pending: { ...model.pending, [id]: pending },
        sendError: without(model.sendError, id),
        error: "",
      };
      // Saved before the request leaves, so a reload mid-flight keeps the ID.
      return [
        next,
        [
          persistUncertain(next),
          request("sendMessage", sendBody(id, pending), id),
        ],
      ];
    }
    case "retrySend": {
      const id = model.active;
      if (!id || !model.uncertain[id] || model.pending[id]) return [model, []];
      const pending = model.uncertain[id]!;
      return [
        {
          ...model,
          pending: { ...model.pending, [id]: pending },
          sendError: without(model.sendError, id),
          error: "",
        },
        [request("sendMessage", sendBody(id, pending), id)],
      ];
    }
    case "discardHeld": {
      const id = model.active;
      if (!id || !model.uncertain[id] || model.pending[id]) return [model, []];
      const next = {
        ...model,
        uncertain: without(model.uncertain, id),
        sendError: without(model.sendError, id),
      };
      return [next, [persistUncertain(next)]];
    }
    case "attach": {
      const id = model.active;
      const session = model.sessions.find((s) => s.sessionId === id);
      if (!id || !session || !msg.items.length) return [model, []];
      const current = model.attachments[id] ?? [];
      const added: DraftAttachment[] = [];
      const effects: Effect[] = [];
      const errors: string[] = [];
      for (const { id: uploadId, file } of msg.items) {
        if (current.length + added.length >= MAX_ATTACHMENTS) {
          errors.push(`At most ${MAX_ATTACHMENTS} attachments per message.`);
          break;
        }
        if (file.size > MAX_UPLOAD_BYTES) {
          errors.push(`${file.name} is larger than 50 MiB.`);
          continue;
        }
        const contentType = file.type || "application/octet-stream";
        added.push({
          id: uploadId,
          name: file.name || "attachment",
          contentType,
          size: file.size,
          state: "uploading",
        });
        effects.push({ type: "upload", sessionId: id, id: uploadId, file });
        if (isImage(contentType))
          effects.push({
            type: "thumbnail",
            sessionId: id,
            id: uploadId,
            file,
          });
      }
      return [
        {
          ...setAttachments(model, id, [...current, ...added]),
          error: errors.join(" "),
        },
        effects,
      ];
    }
    case "uploaded": {
      const list = model.attachments[msg.sessionId] ?? [];
      if (!list.some((a) => a.id === msg.id)) return [model, []];
      const change: Pick<DraftAttachment, "state" | "error"> =
        msg.outcome.kind === "replied"
          ? { state: "ready" }
          : {
              state: "failed",
              error:
                msg.outcome.kind === "rejected"
                  ? msg.outcome.error.message
                  : "Upload outcome unknown. Retry to confirm.",
            };
      const next = setAttachments(
        model,
        msg.sessionId,
        list.map((a) => {
          if (a.id !== msg.id) return a;
          const { error: _, ...rest } = a;
          return { ...rest, ...change };
        }),
      );
      return [next, [persistAttachments(next)]];
    }
    case "retryUpload": {
      const sessionId = model.active;
      const list = sessionId ? (model.attachments[sessionId] ?? []) : [];
      if (
        !sessionId ||
        !list.some((a) => a.id === msg.id && a.state === "failed")
      )
        return [model, []];
      return [
        setAttachments(
          model,
          sessionId,
          list.map((a) => {
            if (a.id !== msg.id) return a;
            const { error: _, ...rest } = a;
            return { ...rest, state: "uploading" as const };
          }),
        ),
        [{ type: "upload", sessionId, id: msg.id }],
      ];
    }
    case "removeAttachment": {
      const sessionId = model.active;
      const list = sessionId ? (model.attachments[sessionId] ?? []) : [];
      if (!sessionId || !list.some((a) => a.id === msg.id)) return [model, []];
      const thumbs = { ...model.thumbs };
      const url = thumbs[msg.id];
      delete thumbs[msg.id];
      // A held send that named this file can no longer be replayed unchanged.
      // Dropping it means the next send uses a new UUID; the held one may still have arrived.
      const held = model.uncertain[sessionId]?.attachments.includes(msg.id);
      const uncertain = { ...model.uncertain };
      if (held) delete uncertain[sessionId];
      const next = {
        ...setAttachments(
          model,
          sessionId,
          list.filter((a) => a.id !== msg.id),
        ),
        thumbs,
        uncertain,
        sendError: held
          ? {
              ...model.sendError,
              [sessionId]:
                "The earlier send was dropped because its file was removed. It may still have arrived.",
            }
          : model.sendError,
      };
      return [
        next,
        [
          { type: "discard", id: msg.id, ...(url ? { url } : {}) },
          persistAttachments(next),
          ...(held ? [persistUncertain(next)] : []),
        ],
      ];
    }
    case "thumb": {
      if (!thumbnailIds(model).has(msg.id))
        return [model, [{ type: "discard", id: msg.id, url: msg.url }]];
      const old = model.thumbs[msg.id];
      return [
        { ...model, thumbs: { ...model.thumbs, [msg.id]: msg.url } },
        old && old !== msg.url
          ? [{ type: "discard", id: msg.id, url: old }]
          : [],
      ];
    }
    case "download":
      return model.active
        ? [
            model,
            [
              {
                type: "download",
                sessionId: model.active,
                id: msg.id,
                name: msg.name,
              },
            ],
          ]
        : [model, []];
    case "retryMessage":
      return model.active
        ? [
            model,
            [
              request(
                "retryMessage",
                { sessionId: model.active, messageId: msg.uuid },
                model.active,
              ),
            ],
          ]
        : [model, []];
    case "openDirectories": {
      const sequence = model.directoryRequest + 1;
      return [
        {
          ...model,
          directoryRequest: sequence,
          sidebarOpen: false,
          drawerOpen: false,
          directoryPicker: {
            listing: model.directoryPicker?.listing ?? null,
            loading: true,
            error: "",
          },
        },
        [
          request("listDirectories", {}, `directory:${sequence}`),
          { type: "directoryFocus", open: true },
        ],
      ];
    }
    case "browseDirectory": {
      if (!model.directoryPicker) return [model, []];
      const sequence = model.directoryRequest + 1;
      return [
        {
          ...model,
          directoryRequest: sequence,
          directoryPicker: {
            ...model.directoryPicker,
            loading: true,
            error: "",
          },
        },
        [
          request(
            "listDirectories",
            { path: msg.path },
            `directory:${sequence}`,
          ),
        ],
      ];
    }
    case "closeDirectories":
      return [
        { ...model, directoryPicker: null },
        [{ type: "directoryFocus", open: false }],
      ];
    case "chooseDirectory": {
      const picker = model.directoryPicker;
      return picker?.listing && !picker.loading
        ? [
            { ...model, directoryPicker: null },
            [
              { type: "directoryFocus", open: false },
              request(
                "addProject",
                { projectId: msg.id, path: picker.listing.path },
                msg.id,
              ),
            ],
          ]
        : [model, []];
    }
    case "promptProjectRename": {
      const project = model.projects.find((p) => p.projectId === msg.projectId);
      return project
        ? [
            model,
            [
              {
                type: "promptProjectRename",
                projectId: project.projectId,
                name: project.name,
              },
            ],
          ]
        : [model, []];
    }
    case "renameProject": {
      const name = msg.name.trim();
      if (!name || [...name].length > limits.maxProjectNameChars)
        return [
          {
            ...model,
            error: `The project name must contain 1 to ${limits.maxProjectNameChars} characters. Enter a shorter, nonblank name.`,
          },
          [],
        ];
      return [
        model,
        [
          request(
            "renameProject",
            { projectId: msg.projectId, name },
            msg.projectId,
          ),
        ],
      ];
    }
    case "requestRemoveProject": {
      const project = model.projects.find((p) => p.projectId === msg.projectId);
      const chats = model.sessions.filter(
        (s) => s.projectId === msg.projectId && rootChat(s),
      );
      // A project with an active chat is never removed; archive its chats first.
      return project && !chats.some((s) => !s.status.archived)
        ? [
            model,
            [
              {
                type: "confirmRemoveProject",
                projectId: project.projectId,
                name: project.name,
                chats: chats.length,
              },
            ],
          ]
        : [model, []];
    }
    case "removeProject":
      return [
        model,
        [request("removeProject", { projectId: msg.projectId }, msg.projectId)],
      ];
    case "openNewChat": {
      const offered = (m: string | undefined) =>
        model.models.find((option) => option.model === m);
      const last = model.lastChoice;
      const remembered = last && offered(last.model) ? last : null;
      const first = offered(limits.defaultModel) ?? model.models[0];
      if (!first) return [model, []];
      return [
        {
          ...model,
          sidebarOpen: false,
          newChat: {
            projectId: msg.projectId,
            name: "",
            model: remembered?.model ?? first.model,
            effort:
              remembered &&
              offered(remembered.model)?.efforts.includes(
                remembered.effort ?? "",
              )
                ? remembered.effort
                : null,
          },
        },
        [],
      ];
    }
    case "dragStart":
      return [
        {
          ...model,
          dragging:
            !model.reorderState && dragItemExists(model, msg.item)
              ? msg.item
              : null,
          dropTarget: null,
        },
        [],
      ];
    case "dragOver":
      return [
        {
          ...model,
          dropTarget: acceptsDrop(model, msg.target)
            ? { id: msg.target.id, after: msg.after }
            : null,
        },
        [],
      ];
    case "dragLeave":
      return model.dropTarget?.id === msg.id
        ? [{ ...model, dropTarget: null }, []]
        : [model, []];
    case "dragEnd":
      return [{ ...model, dragging: null, dropTarget: null }, []];
    case "drop": {
      const next = { ...model, dragging: null, dropTarget: null };
      const source = model.dragging;
      if (!source || !acceptsDrop(model, msg.target)) return [next, []];
      if (source.kind === "project") {
        const projects = moved(
          model.projects,
          model.projects.findIndex((p) => p.projectId === source.id),
          model.projects.findIndex((p) => p.projectId === msg.target.id),
          msg.after,
        );
        if (projects === model.projects) return [next, []];
        return [
          {
            ...next,
            projects,
            reorderState: "saving",
            orderGeneration: model.orderGeneration + 1,
            error: "",
          },
          [
            request(
              "reorderProjects",
              {
                projectIds: projects.map((p) => p.projectId),
              },
              "",
              model.orderGeneration + 1,
            ),
          ],
        ];
      }
      const chats = model.sessions.filter(
        (s) => s.projectId === source.projectId && rootChat(s),
      );
      const ordered = moved(
        chats,
        chats.findIndex((s) => s.sessionId === source.id),
        chats.findIndex((s) => s.sessionId === msg.target.id),
        msg.after,
      );
      if (ordered === chats) return [next, []];
      let index = 0;
      const sessions = model.sessions.map((s) =>
        s.projectId === source.projectId ? ordered[index++]! : s,
      );
      return [
        {
          ...next,
          sessions,
          reorderState: "saving",
          orderGeneration: model.orderGeneration + 1,
          error: "",
        },
        [
          request(
            "reorderChats",
            {
              projectId: source.projectId,
              sessionIds: ordered.map((s) => s.sessionId),
            },
            "",
            model.orderGeneration + 1,
          ),
        ],
      ];
    }
    // Projects added or removed in another tab raise no chat event, so a tab that comes
    // back to the foreground asks for the list again.
    case "refresh":
      if (model.connectionPending) return [model, []];
      return model.token ? [model, [listRequest(model)]] : [model, []];
    case "submitSecret":
      // The value goes straight into the request; it is not kept in the model.
      return msg.value
        ? [
            model,
            [
              request(
                "submitSecret",
                { requestId: msg.requestId, value: msg.value },
                msg.requestId,
              ),
            ],
          ]
        : [model, []];
    case "declineSecret":
      return [
        model,
        [
          request(
            "submitSecret",
            { requestId: msg.requestId, cancel: true },
            msg.requestId,
          ),
        ],
      ];
    case "toggleMessage": {
      const collapsedMessages = { ...model.collapsedMessages };
      collapsedMessages[msg.key] = !(
        collapsedMessages[msg.key] ??
        msg.collapsed ??
        false
      );
      return [{ ...model, collapsedMessages }, []];
    }
    case "copyText":
      return [
        model,
        [
          {
            type: "copy",
            text: msg.text,
            ...(msg.source ? { source: msg.source } : {}),
          },
        ],
      ];
    case "readAloud":
      return [
        { ...model, speaking: msg.key, speechFailed: null },
        [{ type: "speak", key: msg.key, text: msg.text }],
      ];
    case "stopReading":
      return [{ ...model, speaking: null }, [{ type: "stopSpeech" }]];
    case "speechEnded":
      // A reading that ends after a newer one began must not cancel or blame the newer one.
      return model.speaking === msg.key
        ? [
            {
              ...model,
              speaking: null,
              speechFailed: msg.failed ? msg.key : null,
            },
            [],
          ]
        : [model, []];
    case "pushState":
      return [
        {
          ...model,
          pushState: msg.state,
          ...(msg.error ? { error: msg.error } : {}),
        },
        [],
      ];
    case "reloadPage": {
      if (model.active !== msg.sessionId) return [model, []];
      if (
        Object.values(model.attachments).some((items) =>
          items.some((a) => a.state !== "ready"),
        )
      )
        return [
          {
            ...model,
            error:
              "The page could not refresh because an attachment is not uploaded. Finish or remove the attachment, then request another refresh.",
          },
          [],
        ];
      return [model, [{ type: "reloadPage", model }]];
    }
    case "togglePush":
      return [
        model,
        [
          {
            type: "togglePush",
            enabled: model.pushState === "enabled",
            token: model.token,
          },
        ],
      ];
    case "selectDrawerTab":
      return [
        {
          ...model,
          drawerTab: msg.tab,
          ...(msg.tab === "agents" ? { subagentConversation: null } : {}),
        },
        [],
      ];
    case "backToAgents":
      return [{ ...model, subagentConversation: null }, []];
    case "openSubagent": {
      if (!model.active) return [model, []];
      const agent = chatSubagents(model).find((agent) => agent.key === msg.key);
      if (!agent) return [model, []];
      const generation = model.subagentGeneration + 1;
      const boblm = !!agent.session;
      const selected = {
        ...model,
        drawerTab: "agents" as const,
        subagentGeneration: generation,
        subagentConversation: {
          key: agent.key,
          title: agent.title,
          owner: model.active,
          boblm,
          generation,
          timeline: emptyTimeline(),
          cursor: 0,
          hasOlder: false,
          hasNewer: false,
          following: true,
          loading: !boblm,
          error: "",
        },
      };
      return boblm
        ? step(selected, {
            type: "history",
            direction: "latest",
            sessionId: agent.key,
          })
        : [selected, []];
    }
    case "subagentFollowing": {
      const view = model.subagentConversation;
      if (!view) return [model, []];
      if (view.boblm) {
        const current = model.history[view.key];
        return current && current.following !== msg.value
          ? [
              {
                ...model,
                history: {
                  ...model.history,
                  [view.key]: { ...current, following: msg.value },
                },
              },
              [],
            ]
          : [model, []];
      }
      return view.following !== msg.value
        ? [
            {
              ...model,
              subagentConversation: { ...view, following: msg.value },
            },
            [],
          ]
        : [model, []];
    }
    case "subagentPage": {
      const view = model.subagentConversation;
      if (!view || view.loading) return [model, []];
      if (view.boblm)
        return step(model, {
          type: "history",
          direction: msg.latest ? "latest" : "older",
          sessionId: view.key,
        });
      if (!msg.latest && !view.hasOlder) return [model, []];
      const generation = model.subagentGeneration + 1;
      return [
        {
          ...model,
          subagentGeneration: generation,
          subagentConversation: {
            ...view,
            generation,
            before: msg.latest
              ? undefined
              : Math.max(0, (view.timeline.rows[0]?.seq ?? 1) - 1),
            ...(msg.latest ? { timeline: emptyTimeline(), cursor: 0 } : {}),
            following: msg.latest,
            hasNewer: !msg.latest,
            loading: true,
            error: "",
          },
        },
        [],
      ];
    }
    case "subagentReply": {
      const view = model.subagentConversation;
      if (model.forgotten[msg.owner] || model.forgotten[msg.key])
        return [model, []];
      if (
        !view ||
        view.generation !== msg.generation ||
        view.owner !== msg.owner ||
        view.key !== msg.key ||
        model.active !== msg.owner
      )
        return [model, []];
      if (msg.outcome.kind !== "replied") {
        const cause =
          msg.outcome.kind === "rejected"
            ? msg.outcome.error.message
            : (msg.outcome.reason ??
              "The server did not reply. Keep this view open to check again.");
        return [
          {
            ...model,
            subagentConversation: { ...view, loading: false, error: cause },
          },
          [],
        ];
      }
      const page = msg.outcome.reply;
      if (
        page.sessionId !== view.owner ||
        page.agentKey !== view.key ||
        page.events.some((e) => e.sessionId !== view.owner)
      )
        return [
          {
            ...model,
            subagentConversation: {
              ...view,
              loading: false,
              error:
                "The transcript reply did not match this subagent. Return to the agent list and open it again.",
            },
          },
          [],
        ];
      if (!view.loading && !view.error && page.cursor === view.cursor)
        return [model, []];
      const agent = agentName(
        model.sessions.find((s) => s.sessionId === view.owner)?.agent ??
          "claude",
      );
      const events = page.events.map((e) => {
        if (
          e.kind !== "agent_event" ||
          !e.payload ||
          typeof e.payload !== "object" ||
          !("transcript_timestamp" in e.payload)
        )
          return e;
        const timestamp =
          typeof e.payload.transcript_timestamp === "string"
            ? Date.parse(e.payload.transcript_timestamp)
            : NaN;
        return Number.isFinite(timestamp) ? { ...e, createdAt: timestamp } : e;
      });
      const incoming = projectEvents(
        { ...emptyTimeline(), toolNames: view.timeline.toolNames },
        events,
        agent,
        page.cursor,
      );
      // Provider transcripts include user prompts, while the parent event stream normally stores them separately.
      const userSequences = new Set(
        page.events
          .filter(
            (e) =>
              e.kind === "agent_event" &&
              e.payload &&
              typeof e.payload === "object" &&
              !Array.isArray(e.payload) &&
              "type" in e.payload &&
              e.payload.type === "user",
          )
          .map((e) => e.seq),
      );
      incoming.rows = incoming.rows.map((row) =>
        userSequences.has(row.seq) && row.text && !row.tools?.length
          ? { ...row, role: "user" }
          : row,
      );
      let timeline = {
        ...mergeTimeline(view.timeline, incoming),
        toolNames: incoming.toolNames,
      };
      const held =
        view.before === undefined &&
        !view.following &&
        timeline.rows.length > limits.historyMaxRows;
      const keys = new Set(view.timeline.rows.map((row) => row.key));
      const rows = held
        ? timeline.rows.filter((row) => keys.has(row.key))
        : view.before === undefined
          ? timeline.rows.slice(-limits.historyMaxRows)
          : timeline.rows.slice(0, limits.historyMaxRows);
      timeline = { ...timeline, rows };
      return [
        {
          ...model,
          subagentConversation: {
            ...view,
            timeline,
            hasNewer: view.hasNewer || held,
            cursor: page.cursor,
            hasOlder:
              page.hasMore ||
              (view.before === undefined &&
                timeline.rows.length >= limits.historyMaxRows),
            loading: false,
            error: "",
          },
        },
        [],
      ];
    }
    case "questionChoice": {
      const q = model.questions.find(
        (q) =>
          q.id === msg.id &&
          q.sessionId === model.active &&
          q.state === "pending",
      );
      if (!q || Object.keys(model.questionSending).length > 0)
        return [model, []];
      if (typeof msg.choice === "number" && q.choices[msg.choice] === undefined)
        return [model, []];
      const next = {
        ...model,
        questionChoices: { ...model.questionChoices, [msg.id]: msg.choice },
      };
      if (msg.choice === null) return [next, []];
      return msg.choice === "discuss_later"
        ? step(next, { type: "discussQuestionLater", id: msg.id })
        : step(next, {
            type: "answerQuestion",
            id: msg.id,
            answer: q.choices[msg.choice]!,
          });
    }
    case "discussQuestionLater": {
      const q = model.questions.find(
        (q) =>
          q.id === msg.id &&
          q.sessionId === model.active &&
          q.state === "pending",
      );
      if (!q || Object.keys(model.questionSending).length > 0)
        return [model, []];
      return [
        { ...model, questionSending: { [msg.id]: true }, questionError: "" },
        [
          request(
            "discussQuestionLater",
            { sessionId: model.active, questionId: msg.id },
            model.active!,
          ),
        ],
      ];
    }
    case "questionDraft":
      return [
        {
          ...model,
          questionDrafts: { ...model.questionDrafts, [msg.id]: msg.text },
        },
        [],
      ];
    case "answerQuestion": {
      const q = model.questions.find(
        (q) => q.id === msg.id && q.sessionId === model.active,
      );
      if (
        !q ||
        q.state !== "pending" ||
        Object.keys(model.questionSending).length > 0 ||
        !msg.answer.trim()
      )
        return [model, []];
      return [
        {
          ...model,
          questionSending: { ...model.questionSending, [msg.id]: true },
          questionError: "",
        },
        [
          request(
            "answerQuestion",
            { sessionId: model.active, questionId: msg.id, answer: msg.answer },
            model.active!,
          ),
        ],
      ];
    }
    case "reloadQuestions": {
      if (!model.active) return [model, []];
      const generation = model.questionGeneration + 1;
      return [
        { ...model, questionGeneration: generation },
        [
          request(
            "listQuestions",
            { sessionId: model.active },
            model.active,
            generation,
          ),
        ],
      ];
    }
    case "openImage": {
      // Only a picture that has loaded can be shown.
      if (!model.thumbs[msg.id] || !model.active) return [model, []];
      const found = lightboxImages(model).find((image) => image.id === msg.id);
      if (!found) return [model, []];
      return [
        {
          ...model,
          lightbox: found,
        },
        [{ type: "focusLightbox" }],
      ];
    }
    case "stepImage": {
      if (!model.lightbox) return [model, []];
      const images = lightboxImages(model);
      const index = images.findIndex(
        (image) => image.id === model.lightbox?.id,
      );
      const next = images[index + msg.direction];
      return index >= 0 && next
        ? [
            {
              ...model,
              lightbox: {
                ...next,
                returnId: model.lightbox.returnId ?? model.lightbox.id,
              },
            },
            [],
          ]
        : [model, []];
    }
    case "closeImage":
      return model.lightbox
        ? [
            { ...model, lightbox: null },
            [
              {
                type: "focusImage",
                id: model.lightbox.returnId ?? model.lightbox.id,
              },
            ],
          ]
        : [model, []];
    case "backdropClick":
      // A click on the picture or its buttons is not a click outside.
      return msg.onImage ? [model, []] : step(model, { type: "closeImage" });
    case "toggleSidebar":
      return [
        { ...model, sidebarOpen: !model.sidebarOpen, drawerOpen: false },
        [],
      ];
    case "toggleDrawer": {
      const open = !model.drawerOpen;
      return [{ ...model, drawerOpen: open }, []];
    }
    case "dismissUncertain": {
      const dismissed = { ...model.dismissed, [msg.uuid]: true as const };
      return [
        { ...model, dismissed },
        [{ type: "persistDismissed", dismissed }],
      ];
    }
    case "sendAgain": {
      if (!model.active) return [model, []];
      // Put the text back for the owner to review; nothing is sent, because the
      // original may have arrived and its attachments are not carried over.
      const typed = model.drafts[model.active] ?? "";
      const text = typed ? `${typed}\n\n${msg.text}` : msg.text;
      const drafts = { ...model.drafts, [model.active]: text };
      const dismissed = { ...model.dismissed, [msg.uuid]: true as const };
      return [
        { ...model, drafts, dismissed },
        [
          { type: "persist", token: model.token, drafts },
          { type: "composer", sessionId: model.active, text },
          { type: "persistDismissed", dismissed },
        ],
      ];
    }
    case "toggleProject": {
      // A project with no chats stays folded until it gets one.
      if (
        !model.sessions.some(
          (s) => s.projectId === msg.projectId && rootChat(s),
        )
      )
        return [model, []];
      const collapsed = { ...model.collapsed };
      if (collapsed[msg.projectId]) delete collapsed[msg.projectId];
      else collapsed[msg.projectId] = true;
      return [
        { ...model, collapsed },
        [{ type: "persistCollapsed", collapsed }],
      ];
    }
    case "jump": {
      const target = visibleChats(model)[msg.number - 1];
      return target
        ? step(model, { type: "select", id: target.sessionId })
        : [model, []];
    }
    case "closeNewChat":
      return [{ ...model, newChat: null }, []];
    case "newChatName":
      return model.newChat
        ? [{ ...model, newChat: { ...model.newChat, name: msg.name } }, []]
        : [model, []];
    case "requestDelete": {
      const chat = model.sessions.find((s) => s.sessionId === model.active);
      return chat?.status.archived
        ? [
            model,
            [
              {
                type: "confirmDelete",
                sessionId: chat.sessionId,
                name: chat.name || "Untitled chat",
              },
            ],
          ]
        : [model, []];
    }
    case "requestDeleteChat": {
      const chat = model.sessions.find((s) => s.sessionId === msg.sessionId);
      return chat?.status.archived
        ? [
            model,
            [
              {
                type: "confirmDelete",
                sessionId: chat.sessionId,
                name: chat.name || "Untitled chat",
              },
            ],
          ]
        : [model, []];
    }
    case "deleteChat":
      return model.sessions.some(
        (s) => s.sessionId === msg.sessionId && s.status.archived,
      )
        ? [
            model,
            [
              request(
                "deleteSession",
                { sessionId: msg.sessionId },
                msg.sessionId,
              ),
            ],
          ]
        : [model, []];
    case "openRename": {
      const session = model.sessions.find((s) => s.sessionId === model.active);
      return session
        ? [
            {
              ...model,
              renameChat: {
                sessionId: session.sessionId,
                name: session.name,
                saving: false,
                error: "",
              },
            },
            [],
          ]
        : [model, []];
    }
    case "renameName":
      return model.renameChat && !model.renameChat.saving
        ? [
            { ...model, renameChat: { ...model.renameChat, name: msg.name } },
            [],
          ]
        : [model, []];
    case "closeRename":
      return model.renameChat?.saving
        ? [model, []]
        : [{ ...model, renameChat: null }, []];
    case "saveRename": {
      const chat = model.renameChat;
      if (!chat || chat.saving || !chat.name.trim()) return [model, []];
      return [
        { ...model, renameChat: { ...chat, saving: true, error: "" } },
        [
          request(
            "renameSession",
            { sessionId: chat.sessionId, name: chat.name.trim() },
            chat.sessionId,
          ),
        ],
      ];
    }
    case "newChatModel": {
      if (!model.newChat) return [model, []];
      const efforts =
        model.models.find((option) => option.model === msg.model)?.efforts ??
        [];
      const keep =
        model.newChat.effort !== null && efforts.includes(model.newChat.effort);
      return [
        {
          ...model,
          newChat: {
            ...model.newChat,
            model: msg.model,
            effort: keep ? model.newChat.effort : null,
          },
        },
        [],
      ];
    }
    case "newChatEffort":
      return model.newChat
        ? [{ ...model, newChat: { ...model.newChat, effort: msg.effort } }, []]
        : [model, []];
    case "createChat": {
      const chat = model.newChat;
      if (!chat) return [model, []];
      const choice = { model: chat.model, effort: chat.effort };
      return [
        { ...model, newChat: null, lastChoice: choice },
        [
          { type: "persistChoice", choice },
          request(
            "createSession",
            {
              sessionId: msg.id,
              projectId: chat.projectId,
              name: chat.name.trim(),
              model: chat.model,
              effort: chat.effort,
            },
            msg.id,
          ),
        ],
      ];
    }
    case "command":
      return model.active
        ? [
            model,
            [request(msg.name, { sessionId: model.active }, model.active)],
          ]
        : [model, []];
    case "commandOn":
      return [
        model,
        [request(msg.name, { sessionId: msg.sessionId }, msg.sessionId)],
      ];
    case "choice": {
      const session = model.sessions.find((s) => s.sessionId === model.active);
      if (!session) return [model, []];
      return [
        model,
        [
          request(
            "setModel",
            {
              sessionId: session.sessionId,
              model: msg.model ?? session.status.model,
              effort:
                msg.effort === undefined
                  ? (session.status.effort ?? null)
                  : msg.effort,
            },
            session.sessionId,
          ),
        ],
      ];
    }
    case "reply": {
      // A command already in flight can be refused after its chat was deleted. That refusal
      // belongs to a chat the browser has forgotten, so it must not replace the current error.
      if (msg.sessionId && model.forgotten[msg.sessionId]) return [model, []];
      if (msg.name === "listQuestions") {
        if (
          msg.sessionId !== model.active ||
          msg.orderGeneration !== model.questionGeneration
        )
          return [model, []];
        if (msg.outcome.kind === "replied")
          return [
            {
              ...model,
              questions: (msg.outcome.reply as QuestionsReply).questions,
              questionPendingCount: (msg.outcome.reply as QuestionsReply)
                .pendingCount,
              questionError: "",
              questionToFocus: null,
            },
            model.questionToFocus
              ? [{ type: "focusQuestion", id: model.questionToFocus }]
              : [],
          ];
        return [
          {
            ...model,
            questionError:
              msg.outcome.kind === "rejected"
                ? msg.outcome.error.message
                : "Questions could not be loaded. Check the connection and choose Try again.",
          },
          [],
        ];
      }
      if (
        msg.name === "answerQuestion" ||
        msg.name === "discussQuestionLater"
      ) {
        if (msg.sessionId !== model.active) return [model, []];
        const error =
          msg.outcome.kind === "rejected"
            ? msg.outcome.error.message
            : msg.outcome.kind === "uncertain"
              ? "The question reply was lost. Choose Try again to check whether your choice was saved before submitting it again."
              : "";
        const choices = { ...model.questionChoices };
        if (msg.outcome.kind !== "replied") {
          for (const id of Object.keys(model.questionSending)) {
            if (choices[id] !== null) delete choices[id];
          }
        }
        const next = {
          ...model,
          questionGeneration: model.questionGeneration + 1,
          questionChoices: choices,
          questionSending: {},
          questionError: error,
        };
        if (msg.outcome.kind === "replied") {
          const replies = (msg.outcome.reply as QuestionsReply).questions;
          return [
            {
              ...next,
              questions: model.questions.map(
                (q) => replies.find((r) => r.id === q.id) ?? q,
              ),
              questionPendingCount: (msg.outcome.reply as QuestionsReply)
                .pendingCount,
            },
            [
              request(
                "listQuestions",
                { sessionId: model.active },
                model.active!,
                next.questionGeneration,
              ),
            ],
          ];
        }
        return [next, []];
      }
      if (msg.name === "deleteSession" && msg.outcome.kind === "replied")
        return forgetChat(model, msg.sessionId);
      if (
        msg.name === "renameSession" &&
        model.renameChat?.sessionId === msg.sessionId
      ) {
        if (msg.outcome.kind === "replied")
          return [{ ...model, renameChat: null }, [listRequest(model)]];
        const error =
          msg.outcome.kind === "rejected"
            ? msg.outcome.error.message
            : "The rename reply was lost. Check the title before saving again.";
        return [
          {
            ...model,
            renameChat: { ...model.renameChat, saving: false, error },
          },
          [listRequest(model)],
        ];
      }
      if (msg.name === "listDirectories") {
        const picker = model.directoryPicker;
        if (!picker || msg.sessionId !== `directory:${model.directoryRequest}`)
          return [model, []];
        if (msg.outcome.kind === "replied")
          return [
            {
              ...model,
              directoryPicker: {
                listing: msg.outcome.reply as DirectoryListing,
                loading: false,
                error: "",
              },
            },
            [],
          ];
        const error =
          msg.outcome.kind === "rejected"
            ? msg.outcome.error.message
            : `The folder list could not be loaded: ${msg.outcome.reason ?? "no reply arrived"}. Check the connection and try opening the folder again.`;
        return [
          { ...model, directoryPicker: { ...picker, loading: false, error } },
          [],
        ];
      }
      if (msg.name === "reorderProjects" || msg.name === "reorderChats") {
        if (
          model.reorderState !== "saving" ||
          msg.orderGeneration !== model.orderGeneration
        )
          return [model, []];
        const settled = {
          ...model,
          orderGeneration: model.orderGeneration + 1,
        };
        if (msg.outcome.kind === "replied")
          return [
            { ...settled, reorderState: "refreshing" },
            [listRequest(settled)],
          ];
        const error =
          msg.outcome.kind === "rejected"
            ? msg.outcome.error.message
            : `No usable reply while saving sidebar order: ${msg.outcome.reason ?? "no reply arrived"}. The order may have been saved. Refreshing the list; the move will not be retried automatically.`;
        return [
          { ...settled, error, reorderState: "refreshing" },
          [listRequest(settled)],
        ];
      }
      if (
        msg.name === "listSessions" &&
        (msg.orderGeneration ?? 0) !== model.orderGeneration
      )
        return [model, []];
      if (msg.name === "listSessions" && model.reorderState === "saving")
        return [model, []];
      if (msg.name === "listSessions" && model.reorderState === "refreshing") {
        if (msg.outcome.kind === "replied") {
          const list = msg.outcome.reply as Pick<
            Model,
            "sessions" | "projects" | "models"
          >;
          return [{ ...model, ...list, reorderState: null }, []];
        }
        const why =
          msg.outcome.kind === "rejected"
            ? msg.outcome.error.message
            : (msg.outcome.reason ?? "no reply arrived");
        return [
          {
            ...model,
            reorderState: null,
            error: `${model.error} The sidebar order could not be refreshed: ${why}. Reconnect and refresh the page before moving another item.`,
          },
          [],
        ];
      }
      if (msg.outcome.kind !== "replied") {
        const why =
          msg.outcome.kind === "uncertain"
            ? (msg.outcome.reason ?? "no reply arrived")
            : "";
        if (msg.name !== "sendMessage")
          return [
            {
              ...model,
              error:
                msg.outcome.kind === "uncertain"
                  ? `No usable reply to ${msg.name}: ${why}. It may or may not have happened. Nothing is retried automatically; check the chat, then try again if needed.`
                  : msg.outcome.error.message,
            },
            [],
          ];
        const message =
          msg.outcome.kind === "uncertain"
            ? `No usable reply from the server: ${why}. We cannot tell whether it received your message. Retry resends it under the same ID, so it will not be delivered twice.`
            : msg.outcome.error.message;
        // A send failure stays with its session and its composer, not the chat header.
        const pending = model.pending[msg.sessionId];
        // An internal server error can follow a saved message, so it is as unsure as a
        // lost reply: keep the UUID and retry under it. Other refusals mean not accepted.
        const unsure =
          msg.outcome.kind === "uncertain" ||
          msg.outcome.error.code === "internal" ||
          msg.outcome.error.code === "unavailable";
        const held = pending && unsure;
        const next = {
          ...model,
          pending: without(model.pending, msg.sessionId),
          sendError: { ...model.sendError, [msg.sessionId]: message },
          ...(held
            ? { uncertain: { ...model.uncertain, [msg.sessionId]: pending } }
            : {}),
        };
        // The in-flight copy is gone either way; keep storage in step.
        return [next, pending ? [persistUncertain(next)] : []];
      }
      const value = msg.outcome.reply as { status?: SessionStatus };
      let next = value.status
        ? status(model, msg.sessionId, value.status)
        : model;
      const effects: Effect[] = [];
      if (msg.name === "listSessions") {
        const list = msg.outcome.reply as {
          cursor?: number;
          sessions: ReadonlyArray<SessionInfo>;
          models: ReadonlyArray<ModelOption>;
          projects: ReadonlyArray<ProjectInfo>;
        };
        next = {
          ...next,
          ...list,
          listCursor: list.cursor ?? 0,
          listGeneration: next.listGeneration + 1,
        };
        effects.push({
          type: "persistList",
          list: {
            sessions: next.sessions,
            projects: next.projects,
            models: next.models,
          },
        });
        // A chat reopened from the remembered list may have been deleted since.
        if (
          next.active &&
          !next.sessions.some((s) => s.sessionId === next.active)
        ) {
          next = {
            ...next,
            active: null,
            forgotten: { ...next.forgotten, [next.active]: true },
          };
          effects.push({ type: "persistLastChat", id: null });
        }
        if (next.restoreChat !== null) {
          // Reopen where the owner was, once, and only if they have not already picked a chat.
          const wanted = next.restoreChat;
          next = { ...next, restoreChat: null };
          if (
            !next.active &&
            next.sessions.some((s) => s.sessionId === wanted)
          ) {
            const [selected, more] = step(next, {
              type: "select",
              id: wanted,
            });
            next = selected;
            effects.push(...more);
          } else if (!next.active)
            effects.push({ type: "persistLastChat", id: null });
        }
      } else if (msg.name === "submitSecret") {
        // A saved or declined request shows in the chat itself; anything else needs saying.
        const answer = msg.outcome.reply as { status: string; message: string };
        if (answer.status !== "saved" && answer.status !== "canceled")
          return [{ ...next, error: answer.message }, effects];
      } else if (msg.name === "addProject") {
        const added = msg.outcome.reply as ProjectInfo;
        next = {
          ...next,
          projects: [
            ...next.projects.filter((p) => p.projectId !== added.projectId),
            added,
          ],
        };
      } else if (msg.name === "renameProject") {
        const renamed = msg.outcome.reply as ProjectInfo;
        next = {
          ...next,
          projects: next.projects.map((p) =>
            p.projectId === renamed.projectId ? renamed : p,
          ),
        };
      } else if (msg.name === "removeProject") {
        // The project's archived chats were deleted with it.
        for (const gone of next.sessions.filter(
          (s) => s.projectId === msg.sessionId,
        )) {
          const [without, more] = forgetChat(next, gone.sessionId);
          next = without;
          effects.push(...more);
        }
        next = {
          ...next,
          projects: next.projects.filter((p) => p.projectId !== msg.sessionId),
        };
      } else if (msg.name === "createSession") {
        const session = msg.outcome.reply as SessionInfo;
        effects.push({ type: "persistLastChat", id: session.sessionId });
        next = {
          ...next,
          sessions: [
            ...next.sessions.filter((s) => s.sessionId !== session.sessionId),
            session,
          ],
          active: session.sessionId,
          subagents: [],
          sidebarOpen: false,
          history: { [session.sessionId]: { ...emptyHistory(), loaded: true } },
          events: {},
          cursors: {},
          collapsed: without(
            next.collapsed,
            session.projectId,
          ) as Model["collapsed"],
        };
        if (session.projectId in model.collapsed)
          effects.push({ type: "persistCollapsed", collapsed: next.collapsed });
        effects.push({
          type: "composer",
          sessionId: session.sessionId,
          text: next.drafts[session.sessionId] ?? "",
        });
      } else if (msg.name === "sendMessage") {
        const pending = next.pending[msg.sessionId];
        const rest = { ...next.pending };
        delete rest[msg.sessionId];
        const uncertain = { ...next.uncertain };
        delete uncertain[msg.sessionId];
        const drafts = { ...next.drafts };
        if (
          pending &&
          !pending.preserveDraft &&
          drafts[msg.sessionId] === pending.content
        ) {
          drafts[msg.sessionId] = "";
          effects.push({
            type: "composer",
            sessionId: msg.sessionId,
            text: "",
          });
        }
        const wasHeld = msg.sessionId in next.uncertain || !!pending;
        next = {
          ...next,
          pending: rest,
          uncertain,
          drafts,
          sendError: without(next.sendError, msg.sessionId),
        };
        if (wasHeld) effects.push(persistUncertain(next));
        if (pending?.attachments.length) {
          const sent = new Set(pending.attachments);
          next = setAttachments(
            next,
            msg.sessionId,
            (next.attachments[msg.sessionId] ?? []).filter(
              (a) => !sent.has(a.id),
            ),
          );
          for (const id of pending.attachments)
            effects.push({ type: "discard", id });
          effects.push(persistAttachments(next));
        }
        effects.push({ type: "persist", token: next.token, drafts });
      }
      return [{ ...next, error: "" }, effects];
    }
    case "event": {
      const e = msg.event;
      if (
        model.forgotten[e.sessionId] ||
        model.hiddenSince !== null ||
        (model.history[e.sessionId] && !model.history[e.sessionId]!.loaded)
      )
        return [model, []];
      if (e.kind === "questions_changed") {
        if (
          e.sessionId !== model.active ||
          e.seq <= (model.cursors[e.sessionId] ?? 0)
        )
          return [model, []];
        return step(
          { ...model, cursors: { ...model.cursors, [e.sessionId]: e.seq } },
          { type: "reloadQuestions" },
        );
      }
      if (e.seq <= (model.cursors[e.sessionId] ?? 0)) return [model, []];
      const current = model.history[e.sessionId] ?? {
        ...emptyHistory(),
        loaded: true,
        timeline: projectEvents(
          emptyTimeline(),
          model.events[e.sessionId] ?? [],
          agentName(
            model.sessions.find((s) => s.sessionId === e.sessionId)?.agent ??
              "claude",
          ),
        ),
      };
      let timeline = projectEvents(
        current.timeline,
        [e],
        agentName(
          model.sessions.find((s) => s.sessionId === e.sessionId)?.agent ??
            "claude",
        ),
      );
      let hasNewer = current.hasNewer;
      let hasOlder = current.hasOlder;
      if (
        current.hasNewer ||
        (!current.following && timeline.rows.length > limits.historyMaxRows)
      ) {
        const keys = new Set(current.timeline.rows.map((row) => row.key));
        timeline = {
          ...timeline,
          rows: timeline.rows.filter((row) => keys.has(row.key)),
        };
        hasNewer = true;
      } else if (timeline.rows.length > limits.historyMaxRows) {
        timeline = {
          ...timeline,
          rows: timeline.rows.slice(-limits.historyMaxRows),
        };
        hasOlder = true;
      }
      const context = model.events[e.sessionId] ?? [];
      const secrets =
        e.kind === "secret_requested"
          ? [
              ...context.filter(
                (old) =>
                  old.kind === "secret_requested" &&
                  old.payload.request_id !== e.payload.request_id,
              ),
              e,
            ]
          : e.kind === "secret_resolved"
            ? context.filter(
                (old) =>
                  old.kind === "secret_requested" &&
                  old.payload.request_id !== e.payload.request_id,
              )
            : context.filter((old) => old.kind === "secret_requested");
      const forActive = e.sessionId === model.active;
      let next: Model = {
        ...model,
        subagents: forActive
          ? subagentsFromEvents([e], model.subagents)
          : model.subagents,
        events: keepViewed(model, model.events, e.sessionId, secrets),
        history: keepViewed(model, model.history, e.sessionId, {
          ...current,
          timeline,
          hasOlder,
          hasNewer,
        }),
        newMessages: forActive
          ? model.newMessages || hasNewer
          : model.newMessages,
        cursors: { ...model.cursors, [e.sessionId]: e.seq },
      };
      if (e.kind === "state_summary")
        next = status(next, e.sessionId, e.payload);
      const effects: Effect[] = [];
      if (e.kind === "message_appended")
        for (const a of e.payload.attachments ?? [])
          if (isImage(a.contentType) && !next.thumbs[a.id])
            effects.push({
              type: "thumbnail",
              sessionId: e.sessionId,
              id: a.id,
            });
      return pruneThumbnails(next, effects);
    }
    case "unknownListEvent":
      return [
        msg.seq <= model.listCursor ? model : { ...model, listCursor: msg.seq },
        [],
      ];
    case "unknownEvent":
      return [
        msg.seq <= (model.cursors[msg.sessionId] ?? 0)
          ? model
          : {
              ...model,
              cursors: { ...model.cursors, [msg.sessionId]: msg.seq },
            },
        [],
      ];
    case "listEvent": {
      const e = msg.event;
      if (e.seq <= model.listCursor) return [model, []];
      let next = { ...model, listCursor: e.seq };
      // A root's sidebar summary (open count, questions, failures) is computed by the
      // server and absent from live events. A child's archive, reopen, delete and turn change
      // refetch the list, as does the server's subagents_changed for what no other event
      // shows (a question asked, answered or removed, a report that failed to arrive).
      const known = model.sessions.find((s) => s.sessionId === e.sessionId);
      const childChanged =
        !!known &&
        (known.parentSessionId != null || known.rootSessionId != null) &&
        (e.kind === "deleted" ||
          e.kind === "archived" ||
          (e.kind === "state_summary" &&
            e.payload.turnActive !== known.status.turnActive));
      if (e.kind === "deleted") {
        const [forgotten, effects] = forgetChat(next, e.sessionId);
        return [
          forgotten,
          childChanged ? [...effects, listRequest(forgotten)] : effects,
        ];
      }
      if (e.kind === "state_summary")
        next = status(next, e.sessionId, {
          ...e.payload,
          // List summaries omit transient predictions. The chat stream owns them.
          promptSuggestion:
            !e.payload.turnActive && e.payload.running
              ? (known?.status.promptSuggestion ?? null)
              : null,
        });
      if (e.kind === "archived")
        next = {
          ...next,
          sessions: next.sessions.map((s) =>
            s.sessionId === e.sessionId
              ? { ...s, status: { ...s.status, archived: e.payload.archived } }
              : s,
          ),
        };
      return [
        next,
        e.kind === "created" ||
        e.kind === "renamed" ||
        e.kind === "subagents_changed" ||
        childChanged
          ? [listRequest(model)]
          : [],
      ];
    }
    case "error":
      return [{ ...model, error: msg.message }, []];
    case "newMessages":
      return model.newMessages === msg.visible
        ? [model, []]
        : [{ ...model, newMessages: msg.visible }, []];
    case "bottom":
      if (model.active && model.history[model.active]?.hasNewer)
        return step(model, { type: "history", direction: "latest" });
      return [
        {
          ...model,
          newMessages: false,
          history:
            model.active && model.history[model.active]
              ? keepViewed(model, model.history, model.active, {
                  ...model.history[model.active]!,
                  following: true,
                })
              : model.history,
        },
        [{ type: "bottom" }],
      ];
  }
}
/**
 * Images an agent showed, one labelled section each: collapsible like a tool call, open to begin with,
 * at full strength. The picture is a thumbnail that opens full screen when tapped.
 */
function agentImages(
  row: {
    text: string;
    attachments?: ReadonlyArray<{
      id: string;
      name: string;
      contentType: string;
    }>;
  },
  model: Model,
) {
  return (row.attachments ?? [])
    .filter((a) => isImage(a.contentType))
    .map((a) => {
      const label = row.text || a.name;
      const url = model.thumbs[a.id];
      return h(
        "details",
        { key: a.id, class: "agent-image", open: true },
        h("summary", {}, `Image: ${label}`),
        url
          ? h(
              "button",
              {
                type: "button",
                class: "image-open",
                "data-image-open": a.id,
                "aria-label": `Open ${label} full screen`,
                onClick: on.clicked({ type: "openImage", id: a.id }),
              },
              h("img", { src: url, alt: label }),
            )
          : h(
              "p",
              { class: "image-loading", role: "status" },
              "Loading image…",
            ),
      );
    });
}
/** Pictures already available in the viewed chat, in timeline order. */
export function lightboxImages(model: Model): { id: string; label: string }[] {
  if (!model.active) return [];
  const rows =
    model.history[model.active]?.timeline.rows ??
    chatRows(model.events[model.active] ?? []);
  const seen = new Set<string>();
  return rows.flatMap((row) =>
    (row.attachments ?? []).flatMap((attachment) => {
      if (
        !isImage(attachment.contentType) ||
        !model.thumbs[attachment.id] ||
        seen.has(attachment.id)
      )
        return [];
      seen.add(attachment.id);
      return [{ id: attachment.id, label: row.text || attachment.name }];
    }),
  );
}
/** One image over the whole page. A click outside the picture, the close button or Escape closes it. */
function lightboxView(
  model: Model,
  open: { id: string; label: string },
  url: string,
) {
  const images = lightboxImages(model);
  const index = images.findIndex((image) => image.id === open.id);
  return h(
    "div",
    {
      class: "lightbox",
      role: "dialog",
      "aria-modal": "true",
      "aria-label": open.label,
      onClick: binding((event) => ({
        type: "backdropClick",
        onImage:
          (event.target as Element | null)?.closest?.("img, button") !== null,
      })),
    },
    h(
      "button",
      {
        type: "button",
        id: "lightbox-close",
        class: "lightbox-close",
        "aria-label": "Close image",
        title: "Close image",
        onClick: on.clicked({ type: "closeImage" }),
      },
      h(
        "svg",
        {
          width: 22,
          height: 22,
          viewBox: "0 0 24 24",
          fill: "none",
          stroke: "currentColor",
          "stroke-width": 2,
          "stroke-linecap": "round",
          "aria-hidden": "true",
        },
        h("path", { d: "M6 6l12 12M18 6 6 18" }),
      ),
    ),
    h("img", { src: url, alt: open.label }),
    h(
      "div",
      { class: "lightbox-footer" },
      images.length > 1
        ? h(
            "button",
            {
              type: "button",
              "aria-label": "Previous image",
              disabled: index <= 0,
              onClick: on.clicked({ type: "stepImage", direction: -1 }),
            },
            "Previous",
          )
        : null,
      h(
        "p",
        {
          class: "lightbox-label",
          "aria-live": "polite",
          "aria-atomic": "true",
        },
        open.label,
        images.length > 1
          ? h(
              "span",
              { class: "lightbox-position" },
              `${index + 1} of ${images.length}`,
            )
          : null,
      ),
      images.length > 1
        ? h(
            "button",
            {
              type: "button",
              "aria-label": "Next image",
              disabled: index >= images.length - 1,
              onClick: on.clicked({ type: "stepImage", direction: 1 }),
            },
            "Next",
          )
        : null,
    ),
  );
}
/** One header toggle controls the shared questions and agents drawer. */
function drawerToggle(open: boolean, pending: number, mobile = false) {
  return h(
    "button",
    {
      type: "button",
      class: `header-icon drawer-toggle${mobile ? " mobile-drawer-toggle" : ""}`,
      "aria-label": open ? "Close drawer" : "Open drawer",
      "aria-expanded": String(open),
      "aria-controls": "chat-drawer",
      title: open ? "Close drawer" : "Open drawer",
      onClick: on.clicked({ type: "toggleDrawer" }),
    },
    h(
      "svg",
      {
        width: 20,
        height: 20,
        viewBox: "0 0 24 24",
        fill: "none",
        stroke: "currentColor",
        "stroke-width": 1.75,
        "aria-hidden": "true",
      },
      h("rect", { x: 3, y: 4, width: 18, height: 16, rx: 2 }),
      h("path", { d: "M15 4v16" }),
    ),
    pending
      ? h(
          "span",
          { class: "count", "aria-label": `${pending} unanswered questions` },
          String(pending),
        )
      : null,
  );
}
function drawerTabs(model: Model, running: number) {
  const tabs = ["agents", "jobs", "questions"] as const;
  const counts = {
    agents: running,
    jobs:
      model.sessions.find((s) => s.sessionId === model.active)?.status.jobs
        ?.tasks.length ?? 0,
    questions: model.questionPendingCount,
  };
  return h(
    "header",
    {},
    h(
      "div",
      { class: "drawer-tabs", role: "tablist", "aria-label": "Chat details" },
      tabs.map((tab) =>
        h(
          "button",
          {
            type: "button",
            role: "tab",
            id: `drawer-tab-${tab}`,
            "aria-selected": String(model.drawerTab === tab),
            "aria-controls": `drawer-panel-${tab}`,
            tabIndex: -1,
            onClick: on.clicked({ type: "selectDrawerTab", tab }),
            onkeydown: binding((event) => {
              const key = (event as KeyboardEvent).key;
              if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(key))
                return { type: "noop" };
              event.preventDefault();
              const next =
                tabs[
                  key === "Home"
                    ? 0
                    : key === "End"
                      ? tabs.length - 1
                      : (tabs.indexOf(tab) +
                          (key === "ArrowRight" ? 1 : tabs.length - 1)) %
                        tabs.length
                ]!;
              (event.currentTarget as HTMLElement).parentElement
                ?.querySelector<HTMLElement>(`#drawer-tab-${next}`)
                ?.focus();
              return { type: "selectDrawerTab", tab: next };
            }),
          },
          tab === "questions"
            ? "Questions"
            : tab === "jobs"
              ? "Jobs"
              : "Subagents",
          counts[tab] ? ` (${counts[tab]})` : "",
        ),
      ),
    ),
    drawerToggle(true, 0, true),
  );
}
/** A small rosette whose petals and hue come from the subagent's id, so each one keeps its look. */
function subagentIcon(key: string) {
  // FNV-1a, then a final mix, so ids that differ in one character still look different.
  let hash = 2166136261;
  for (const ch of key) hash = Math.imul(hash ^ ch.charCodeAt(0), 16777619);
  hash = Math.imul(hash ^ (hash >>> 16), 0x45d9f3b);
  hash = (hash ^ (hash >>> 16)) >>> 0;
  const petals = 5 + (hash % 4);
  const hue = (hash >>> 4) % 360;
  return h(
    "svg",
    { width: 22, height: 22, viewBox: "-11 -11 22 22", "aria-hidden": "true" },
    Array.from({ length: petals }, (_, i) =>
      h("ellipse", {
        key: i,
        cx: 0,
        cy: -5,
        rx: 2.6,
        ry: 4.6,
        transform: `rotate(${(i * 360) / petals})`,
        fill: `hsl(${hue} 55% 68%)`,
      }),
    ),
    h("circle", { r: 2.2, fill: `hsl(${hue} 55% 38%)` }),
  );
}
function chatSubagents(model: Model): ReadonlyArray<Subagent> {
  if (!model.active) return [];
  const known = model.subagents.length
    ? model.subagents
    : subagentsFrom(
        model.history[model.active]?.timeline.rows ?? [],
        subagentsFromEvents(model.events[model.active] ?? [], model.subagents),
      );
  const merged = subagentsFromSessions(model.sessions, model.active, known);
  const jobs = model.sessions.find((s) => s.sessionId === model.active)?.status
    .jobs;
  const waiting = new Set(
    jobs?.tasks.map((job) => job.agentKey).filter(Boolean),
  );
  return merged.map((agent) =>
    agent.state === "done" && waiting.has(agent.key)
      ? { ...agent, state: "waiting" }
      : agent,
  );
}
function jobsContent(model: Model) {
  const snapshot = model.sessions.find((s) => s.sessionId === model.active)
    ?.status.jobs;
  const agents = chatSubagents(model);
  const tasks = snapshot?.tasks ?? [];
  return h(
    "section",
    { class: "subagents-body jobs-body" },
    h("h3", {}, `Running · ${tasks.length}`),
    !snapshot || !snapshot.available
      ? h(
          "p",
          { role: "status" },
          "Background job status is unavailable. Check the agent before restarting.",
        )
      : null,
    snapshot?.available && !tasks.length
      ? h("p", { class: "empty" }, "No running background jobs")
      : null,
    h(
      "ul",
      {},
      tasks
        .slice(0, limits.jobsVisibleLimit)
        .map((job) =>
          h(
            "li",
            { key: job.id },
            h(
              "strong",
              {},
              job.agentKey
                ? (agents.find((agent) => agent.key === job.agentKey)?.title ??
                    "Subagent")
                : "This chat",
            ),
            h("code", {}, job.command),
            h(
              "small",
              {},
              snapshot?.available ? "Running" : "Last seen running",
            ),
          ),
        ),
    ),
    tasks.length > limits.jobsVisibleLimit
      ? h(
          "p",
          {},
          `Showing ${limits.jobsVisibleLimit} of ${tasks.length} running jobs.`,
        )
      : null,
  );
}
/** The right-hand list of the viewed chat's subagents: running ones, then finished ones. */
function agentsContent(subagents: ReadonlyArray<Subagent>, model: Model) {
  const active = subagents.filter(
    (s) => s.state === "active" || s.state === "waiting",
  );
  const done = subagents.filter(
    (s) => s.state !== "active" && s.state !== "waiting",
  );
  const list = (items: ReadonlyArray<Subagent>) =>
    h(
      "ul",
      {},
      items.map((s) => {
        const running =
          s.busy ?? (s.state === "active" || s.state === "waiting");
        return h(
          "li",
          { key: s.key, class: s.state },
          h(
            "button",
            {
              type: "button",
              class: "subagent-open quiet",
              "aria-label": `View ${s.title}`,
              onClick: on.clicked({ type: "openSubagent", key: s.key }),
            },
            subagentIcon(s.key),
            h("span", { class: "title" }, s.title),
            running
              ? h(
                  "span",
                  {
                    class: `activity-indicator running agent-${model.sessions.find((chat) => chat.sessionId === model.active)?.agent ?? "claude"}`,
                    role: "img",
                    "aria-label": "Running",
                  },
                  [0, 1, 2].map((i) =>
                    h("span", { key: i, "aria-hidden": "true" }),
                  ),
                )
              : null,
            s.state === "waiting" ? h("small", {}, "Waiting for jobs") : null,
            s.state === "failed"
              ? h("small", { class: "failed" }, "failed")
              : null,
            h(
              "time",
              {
                datetime: new Date(
                  s.state === "waiting"
                    ? s.startedAt
                    : (s.endedAt ?? s.startedAt),
                ).toISOString(),
                title:
                  s.state === "active" || s.state === "waiting"
                    ? "Started"
                    : "Finished",
              },
              messageTime(
                s.state === "waiting"
                  ? s.startedAt
                  : (s.endedAt ?? s.startedAt),
              ),
            ),
          ),
        );
      }),
    );
  return h(
    "div",
    { class: "subagents-body" },
    h("h3", {}, `Active · ${active.length}`),
    active.length
      ? list(active)
      : h("p", { class: "empty" }, "No active subagents"),
    h("h3", {}, `Done · ${done.length}`),
    list(done),
  );
}
/** Why an expired message never reached the agent; an unconfirmed one may have arrived anyway. */
function expiredNote(from: string | undefined): string {
  const unsent = `Expired unsent after ${limits.queuedMessageMaxAgeHours} hours.`;
  return from === "uncertain"
    ? `${unsent} The agent never confirmed it, so it may already have been received.`
    : unsent;
}
/** A held queue waits for the owner to resume it, so the sidebar must not call it queued. */
function queueWord(status: SessionInfo["status"]): "held" | "queued" {
  return status.queueHeld ? "held" : "queued";
}
function subagentStatusText(session: SessionInfo): string {
  const status = session.status;
  if (status.archived) return "archived";
  if (status.stopping) return "stopping";
  if (status.turnActive) return "working";
  if (status.dispatching > 0) return "dispatching";
  if (status.queued > 0) return status.queueHeld ? "held" : "queued";
  if (status.running) return "idle";
  return "stopped";
}
function subagentConversationContent(model: Model) {
  const view = model.subagentConversation!;
  const child = view.boblm
    ? model.sessions.find((session) => session.sessionId === view.key)
    : undefined;
  const history = view.boblm ? model.history[view.key] : undefined;
  const timeline = history?.timeline ?? view.timeline;
  const rows = groupToolRows(timeline.rows);
  const loading = history?.loading ?? view.loading;
  const error = history?.error ?? view.error;
  const hasOlder = history?.hasOlder ?? view.hasOlder;
  const hasNewer = history?.hasNewer ?? view.hasNewer;
  const following = history?.following ?? view.following;
  const statusText = child ? subagentStatusText(child) : "";
  const stopDisabled =
    !child ||
    child.status.stopping ||
    !(
      child.status.turnActive ||
      child.status.dispatching > 0 ||
      (child.status.queued > 0 && !child.status.queueHeld)
    );
  return h(
    "section",
    { class: "subagent-conversation" },
    h(
      "header",
      { class: "subagent-titlebar" },
      h(
        "button",
        {
          type: "button",
          class: "header-icon",
          "aria-label": "Back to subagents",
          title: "Back to subagents",
          onClick: on.clicked({ type: "backToAgents" }),
        },
        h(
          "svg",
          {
            viewBox: "0 0 24 24",
            width: 18,
            height: 18,
            fill: "none",
            stroke: "currentColor",
            "stroke-width": 2,
            "aria-hidden": "true",
          },
          h("path", { d: "m14 6-6 6 6 6M8 12h13" }),
        ),
      ),
      h(
        "div",
        { class: "subagent-heading" },
        h("strong", {}, view.title),
        child
          ? h(
              "small",
              { class: "subagent-meta" },
              `${child.role ?? "subagent"} · ${modelLabel(model.models, child.status.model)} · ${child.status.effort ?? "default"} · ${statusText}`,
            )
          : null,
      ),
      child
        ? h(
            "div",
            { class: "subagent-actions" },
            h(
              "button",
              {
                type: "button",
                class: "header-icon",
                "aria-label": "Stop",
                title: "Stop",
                disabled: stopDisabled,
                onClick: on.clicked({
                  type: "commandOn",
                  name: "stopSession",
                  sessionId: view.key,
                }),
              },
              composerIcon("stop"),
            ),
            h(
              "button",
              {
                type: "button",
                class: "header-icon",
                "aria-label": child.status.archived ? "Reopen" : "Archive",
                title: child.status.archived ? "Reopen" : "Archive",
                onClick: on.clicked({
                  type: "commandOn",
                  name: child.status.archived
                    ? "reopenSession"
                    : "archiveSession",
                  sessionId: view.key,
                }),
              },
              chatActionIcon(child.status.archived ? "reopen" : "archive"),
            ),
          )
        : null,
    ),
    h(
      "div",
      { class: "messages-frame" },
      h(
        "div",
        {
          id: "subagent-messages",
          class: "messages",
          tabindex: -1,
          "aria-label": `${view.title} conversation`,
          "data-session-id": view.key,
          "data-history-loading": String(loading),
          "data-has-older": String(hasOlder),
          "data-has-newer": String(hasNewer),
        },
        error
          ? h(
              "div",
              { class: "history-status" },
              h("p", { role: "alert" }, error),
              h(
                "button",
                {
                  type: "button",
                  onClick: on.clicked({
                    type: "subagentPage",
                    latest: view.boblm || view.before === undefined,
                  }),
                },
                "Try again",
              ),
            )
          : null,
        loading
          ? h(
              "p",
              { class: "history-status", role: "status" },
              "Loading messages…",
            )
          : null,
        hasOlder
          ? h(
              "button",
              {
                type: "button",
                class: "history-page",
                disabled: loading,
                onClick: on.clicked({ type: "subagentPage", latest: false }),
              },
              "Load older messages",
            )
          : null,
        rows.map((row) =>
          h(
            "article",
            {
              key: `${view.key}:${row.key}`,
              class: `message ${row.role}${model.collapsedMessages[`${model.active}:subagent:${view.key}:${row.key}`] ? " message-collapsed" : ""}`,
              "data-row-key": row.key,
              "data-row-keys": JSON.stringify(row.sourceKeys ?? [row.key]),
            },
            model.collapsedMessages[
              `${model.active}:subagent:${view.key}:${row.key}`
            ]
              ? h(
                  "div",
                  { class: "message-preview" },
                  row.text.replace(/\s+/g, " ").trim(),
                )
              : null,
            messageBody(row),
            row.text
              ? messageActions(
                  {
                    ...row,
                    key: `subagent:${view.key}:${row.key}`,
                    ...(row.at ? { at: row.at } : {}),
                  },
                  model,
                )
              : null,
          ),
        ),
        !loading && !error && !rows.length
          ? h(
              "p",
              { class: "empty" },
              "This subagent has not written any messages yet.",
            )
          : null,
      ),
      following && !hasNewer
        ? scrollLock()
        : h(
            "button",
            {
              id: "subagent-latest",
              class: "jump-latest",
              "aria-label": "Jump to latest subagent messages",
              title: "Jump to latest messages",
              disabled: loading,
              onClick: on.clicked({ type: "subagentPage", latest: true }),
            },
            composerIcon("send"),
          ),
    ),
  );
}

/**
 * A small sign under the message scrollbar, not a control: a closed lock while the chat follows the
 * newest messages. Once the owner has scrolled away and the view stays put, nothing is shown.
 */
function scrollLock() {
  return h(
    "span",
    {
      class: "scroll-lock",
      role: "img",
      "data-state": "following",
      "aria-label": "Following new messages",
      title: "Following new messages",
    },
    h(
      "svg",
      {
        width: 12,
        height: 12,
        viewBox: "0 0 24 24",
        fill: "none",
        stroke: "currentColor",
        "stroke-width": 2.2,
        "stroke-linecap": "round",
        "stroke-linejoin": "round",
        "aria-hidden": "true",
      },
      h("rect", { x: 5, y: 11, width: 14, height: 10, rx: 2 }),
      h("path", { d: "M8 11V7a4 4 0 0 1 8 0v4" }),
    ),
  );
}
/** Shared text and tool rendering for the main chat and the subagent conversation. */
function messageBody(row: VisibleRow) {
  return [
    row.text && !(row.role === "assistant" && row.attachments?.length)
      ? row.role === "assistant" && !row.senderSessionId
        ? h("div", {
            class: "markdown",
            innerHTML: renderMarkdown(row.text),
            onClick: binding(copyCodeClick),
          })
        : h("pre", {}, row.text)
      : null,
    row.toolGroupLabel
      ? h(
          "details",
          { class: "tool-details tool-group" },
          h("summary", {}, row.toolGroupLabel),
          row.tools?.map((tool) =>
            h(
              "details",
              { key: tool.key, class: "tool-details" },
              h("summary", {}, tool.label),
              h("pre", {}, tool.text),
            ),
          ),
        )
      : row.tools?.map((tool) =>
          h(
            "details",
            {
              key: tool.key,
              class: `tool-details${tool.error ? " tool-error" : ""}`,
            },
            h("summary", {}, tool.label),
            h("pre", {}, tool.text),
          ),
        ),
  ];
}
/** Peer messages start folded; an explicit choice wins across later updates. */
function isMessageCollapsed(
  model: Model,
  key: string,
  senderSessionId?: string,
) {
  return model.collapsedMessages[key] ?? !!senderSessionId;
}

/** The quiet row under a message: copy its raw text, and when it was sent. */
function messageActions(
  row: {
    key: string;
    role: string;
    text: string;
    at?: number;
    senderSessionId?: string;
  },
  model: Model,
) {
  const readable =
    row.role === "assistant" && !row.senderSessionId && supportsSpeech()
      ? speechText(row.text)
      : "";
  const reading = model.speaking === row.key;
  const messageKey = `${model.active}:${row.key}`;
  const folded = isMessageCollapsed(model, messageKey, row.senderSessionId);
  const sent =
    row.at !== undefined && Number.isFinite(row.at) ? new Date(row.at) : null;
  return h(
    "div",
    {
      class: `message-actions${reading || model.speechFailed === row.key ? " pinned" : ""}`,
    },
    h(
      "button",
      {
        type: "button",
        class: "action-button",
        "aria-label": "Copy message",
        title: "Copy message",
        onClick: binding((event) => {
          const button = (event.target as Element | null)?.closest?.("button");
          return {
            type: "copyText",
            text: row.text,
            ...(button ? { source: button as HTMLElement } : {}),
          };
        }),
      },
      actionIcon("copy"),
    ),
    readable
      ? h(
          "button",
          {
            type: "button",
            class: "action-button",
            "aria-label": reading ? "Stop reading" : "Read aloud",
            "aria-pressed": String(reading),
            title: reading ? "Stop reading" : "Read aloud",
            onClick: on.clicked(
              reading
                ? { type: "stopReading" }
                : { type: "readAloud", key: row.key, text: readable },
            ),
          },
          actionIcon(reading ? "stop" : "speaker"),
        )
      : null,
    h(
      "button",
      {
        type: "button",
        class: "action-button",
        "aria-label": folded ? "Expand message" : "Collapse message",
        "aria-expanded": String(!folded),
        title: folded ? "Expand message" : "Collapse message",
        onClick: on.clicked({
          type: "toggleMessage",
          key: messageKey,
          collapsed: folded,
        }),
      },
      actionIcon(folded ? "expand" : "collapse"),
    ),
    model.speechFailed === row.key
      ? h(
          "span",
          { role: "status", class: "speech-error" },
          "Could not read this aloud. The browser's speech engine reported an error; try again.",
        )
      : null,
    sent
      ? h("time", { datetime: sent.toISOString() }, messageTime(sent.getTime()))
      : null,
  );
}
function actionIcon(kind: "copy" | "speaker" | "stop" | "expand" | "collapse") {
  return h(
    "svg",
    {
      width: 16,
      height: 16,
      viewBox: "0 0 24 24",
      fill: "none",
      stroke: "currentColor",
      "stroke-width": 2,
      "stroke-linecap": "round",
      "stroke-linejoin": "round",
      "aria-hidden": "true",
    },
    kind === "collapse" || kind === "expand"
      ? h("path", { d: kind === "collapse" ? "m6 15 6-6 6 6" : "m9 6 6 6-6 6" })
      : kind === "copy"
        ? [
            h("rect", { x: 9, y: 9, width: 13, height: 13, rx: 2 }),
            h("path", {
              d: "M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1",
            }),
          ]
        : kind === "speaker"
          ? [
              h("path", { d: "M11 5 6 9H2v6h4l5 4V5Z" }),
              h("path", { d: "M15.5 8.5a5 5 0 0 1 0 7M19 5a10 10 0 0 1 0 14" }),
            ]
          : h("rect", {
              x: 6,
              y: 6,
              width: 12,
              height: 12,
              rx: 1,
              fill: "currentColor",
              stroke: "none",
            }),
  );
}
/** One listener on the message catches clicks on the copy buttons inside rendered code blocks. */
function copyCodeClick(event: Event): Msg {
  const button = (event.target as Element | null)?.closest?.(
    "button.copy-code",
  );
  const code = button?.parentElement?.querySelector("code")?.textContent;
  return button && code != null
    ? { type: "copyText", text: code, source: button as HTMLElement }
    : { type: "noop" };
}
export function chatRows(events: ReadonlyArray<StoredEvent>, agent = "claude") {
  return projectEvents(emptyTimeline(), events, agentName(agent)).rows.map(
    ({ seq: _seq, revision: _revision, ...row }) => row,
  );
}
function binding(toMsg: (event: Event) => Msg) {
  return { kind: "eventBinding" as const, preventDefault: false, toMsg };
}
/** Native browser mechanics stay at the event boundary; update validates every move. */
function dragProps(model: Model, item: DragItem, className: string) {
  const target = model.dropTarget;
  const over = (event: Event): Extract<Msg, { type: "dragOver" }> => {
    event.stopPropagation();
    if (acceptsDrop(model, item)) {
      event.preventDefault();
      const transfer = (event as DragEvent).dataTransfer;
      if (transfer) transfer.dropEffect = "move";
    }
    const rect = (event.currentTarget as Element).getBoundingClientRect();
    return {
      type: "dragOver",
      target: item,
      after: (event as DragEvent).clientY >= rect.top + rect.height / 2,
    };
  };
  return {
    draggable: !model.reorderState,
    "data-drag-id": item.id,
    class:
      className +
      (model.dragging?.kind === item.kind && model.dragging.id === item.id
        ? " dragging"
        : "") +
      (target?.id === item.id && acceptsDrop(model, item)
        ? target.after
          ? " drop-after"
          : " drop-before"
        : ""),
    onDragStart: binding((event) => {
      event.stopPropagation();
      const button = (event.target as Element).closest("button");
      if (model.reorderState || (button && button !== event.currentTarget)) {
        event.preventDefault();
        return { type: "noop" };
      }
      const transfer = (event as DragEvent).dataTransfer;
      if (transfer) {
        transfer.setData("text/plain", item.id);
        transfer.effectAllowed = "move";
      }
      return { type: "dragStart", item };
    }),
    onDragEnter: binding(over),
    ondragover: binding(over),
    ondragleave: binding((event) => {
      event.stopPropagation();
      const related = (event as DragEvent).relatedTarget as Node | null;
      if (related && (event.currentTarget as Element).contains(related))
        return { type: "noop" };
      return { type: "dragLeave", id: item.id };
    }),
    onDragEnd: on.dragEnded({ type: "dragEnd" }),
    ondrop: binding((event) => {
      const hover = over(event);
      return { type: "drop", target: item, after: hover.after };
    }),
  };
}
function trashIcon(size = 18) {
  return h(
    "svg",
    {
      viewBox: "0 0 24 24",
      width: String(size),
      height: String(size),
      fill: "none",
      stroke: "currentColor",
      "stroke-width": "1.8",
      "aria-hidden": "true",
    },
    h("path", {
      d: "M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7M14 10v7",
    }),
  );
}
function chatActionIcon(kind: "archive" | "reopen") {
  return h(
    "svg",
    {
      class: "chat-action-icon",
      viewBox: "0 0 24 24",
      width: 18,
      height: 18,
      fill: "none",
      stroke: "currentColor",
      "stroke-width": 1.8,
      "stroke-linecap": "round",
      "stroke-linejoin": "round",
      "aria-hidden": "true",
    },
    h("path", {
      d:
        kind === "archive"
          ? "M3 3h18v4H3zM5 7v14h14V7M9 11h6"
          : "M4 10a8 8 0 1 1 0 7M4 4v6h6",
    }),
  );
}
function composerIcon(kind: "attach" | "stop" | "send") {
  return h(
    "svg",
    {
      width: 20,
      height: 20,
      viewBox: "0 0 24 24",
      fill: "none",
      stroke: "currentColor",
      "stroke-width": 2,
      "stroke-linecap": "round",
      "stroke-linejoin": "round",
      "aria-hidden": "true",
    },
    kind === "stop"
      ? h("rect", {
          x: 5,
          y: 5,
          width: 14,
          height: 14,
          rx: 1,
          fill: "currentColor",
          stroke: "none",
        })
      : h("path", {
          d:
            kind === "send"
              ? "M12 19V5m-7 7 7-7 7 7"
              : "m21 11-8.5 8.5a6 6 0 0 1-8.5-8.5l9-9a4 4 0 0 1 5.7 5.7l-9 9a2 2 0 0 1-2.8-2.8l8.5-8.5",
        }),
  );
}
function connectionDialog(model: Model): VNode<Model> {
  return h(
    "main",
    { class: "connection-screen" },
    h(
      "div",
      { class: "backdrop" },
      h(
        "form",
        {
          class: "dialog connection-dialog",
          role: "dialog",
          "aria-modal": "true",
          "aria-labelledby": "connection-title",
          "aria-busy": String(model.connectionPending),
          onSubmit: binding((event) => {
            event.preventDefault();
            const form = event.currentTarget as HTMLFormElement;
            return {
              type: "login",
              token: (form.elements.namedItem("token") as HTMLInputElement)
                .value,
            };
          }),
          onkeydown: binding((event) => {
            const key = (event as KeyboardEvent).key;
            if (key === "Escape") {
              event.preventDefault();
              event.stopPropagation();
            }
            if (key === "Tab") {
              const fields = [
                ...(
                  event.currentTarget as HTMLElement
                ).querySelectorAll<HTMLElement>("input, button:not(:disabled)"),
              ];
              const first = fields[0];
              const last = fields.at(-1);
              if ((event as KeyboardEvent).shiftKey && event.target === first) {
                event.preventDefault();
                last?.focus();
              } else if (
                !(event as KeyboardEvent).shiftKey &&
                event.target === last
              ) {
                event.preventDefault();
                first?.focus();
              }
            }
            return { type: "noop" };
          }),
        },
        h("strong", { id: "connection-title" }, "Connect to BobLM"),
        h("label", { for: "token" }, "Bearer token"),
        h("input", {
          id: "token",
          name: "token",
          type: "password",
          required: true,
          autofocus: true,
          autocomplete: "current-password",
          readOnly: model.connectionPending,
        }),
        model.error
          ? h("p", { class: "error", role: "alert" }, model.error)
          : null,
        h(
          "button",
          { type: "submit", disabled: model.connectionPending },
          model.connectionPending ? "Connecting…" : "Connect",
        ),
      ),
    ),
  );
}
export function view(model: Model): VNode<Model> {
  if (!model.token || model.connectionPending) return connectionDialog(model);
  const session = model.sessions.find((s) => s.sessionId === model.active);
  const sending = !!(model.active && model.pending[model.active]);
  const numbers = new Map(
    visibleChats(model).map((s, i) => [s.sessionId, i + 1] as const),
  );
  const rowsOf = session
    ? (model.history[session.sessionId]?.timeline.rows ??
      chatRows(model.events[session.sessionId] ?? [], session.agent))
    : [];
  const subagents = session ? chatSubagents(model) : [];
  const drawerOpen = !!session && model.drawerOpen;
  // The phone's chat list shows when asked for, and always when there is nothing else to show.
  const sidebarShown =
    !model.directoryPicker &&
    (model.sidebarOpen || !model.token || !model.active);
  return h(
    "main",
    {
      class: [
        "shell",
        drawerOpen ? "with-subagents" : "",
        sidebarShown ? "sidebar-open" : "",
      ]
        .filter(Boolean)
        .join(" "),
    },
    h(
      "aside",
      { class: "sidebar" },
      h(
        "header",
        {},
        model.active
          ? h(
              "button",
              {
                type: "button",
                class: "header-icon sidebar-close",
                "aria-label": "Close chat list",
                title: "Close chat list",
                onClick: on.clicked({ type: "toggleSidebar" }),
              },
              h(
                "svg",
                {
                  width: 18,
                  height: 18,
                  viewBox: "0 0 24 24",
                  fill: "none",
                  stroke: "currentColor",
                  "stroke-width": 2,
                  "stroke-linecap": "round",
                  "aria-hidden": "true",
                },
                h("path", { d: "M6 6l12 12M18 6 6 18" }),
              ),
            )
          : null,
        model.token && model.pushState !== "unavailable"
          ? h(
              "button",
              {
                type: "button",
                class: "header-icon notification-toggle",
                "aria-pressed": String(model.pushState === "enabled"),
                "aria-label":
                  model.pushState === "enabled"
                    ? "Disable notifications"
                    : "Enable notifications",
                title:
                  model.pushState === "enabled"
                    ? "Disable notifications"
                    : "Enable notifications",
                onClick: on.clicked({ type: "togglePush" }),
              },
              h(
                "svg",
                {
                  viewBox: "0 0 24 24",
                  width: 20,
                  height: 20,
                  fill: "none",
                  stroke: "currentColor",
                  "stroke-width": 1.75,
                  "stroke-linecap": "round",
                  "stroke-linejoin": "round",
                  "aria-hidden": "true",
                },
                h("path", {
                  d: "M18 8a6 6 0 0 0-12 0c0 7-3 7-3 9h18c0-2-3-2-3-9M10 21h4",
                }),
                model.pushState !== "enabled"
                  ? h("path", { d: "m3 3 18 18" })
                  : null,
              ),
            )
          : null,
        model.token
          ? h(
              "button",
              {
                onClick: on.clicked({ type: "requestLock" }),
                class: "header-icon sidebar-lock",
                "aria-label": "Lock",
                title: "Lock",
              },
              h(
                "svg",
                {
                  viewBox: "0 0 24 24",
                  width: 18,
                  height: 18,
                  fill: "none",
                  stroke: "currentColor",
                  "stroke-width": 2,
                  "aria-hidden": "true",
                },
                h("rect", { x: 5, y: 10, width: 14, height: 11, rx: 2 }),
                h("path", { d: "M8 10V7a4 4 0 0 1 8 0v3" }),
              ),
            )
          : null,
        model.token
          ? h(
              "button",
              {
                type: "button",
                "aria-label": "Browse folders",
                title: "Browse folders",
                class: "header-icon browse-folders",
                onClick: on.clicked({ type: "openDirectories" }),
                "data-browse-folders": true,
              },
              h(
                "svg",
                {
                  viewBox: "0 0 24 24",
                  width: 20,
                  height: 20,
                  fill: "none",
                  stroke: "currentColor",
                  "stroke-width": 1.75,
                  "stroke-linecap": "round",
                  "stroke-linejoin": "round",
                  "aria-hidden": "true",
                  focusable: "false",
                },
                h("path", {
                  d: "M3 7V5a1 1 0 0 1 1-1h5l2 3h9a1 1 0 0 1 1 1v11a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V7Z",
                }),
              ),
            )
          : null,
      ),
      h(
        "div",
        {},
        model.projects.map((project) => {
          const chats = model.sessions.filter(
            (s) => s.projectId === project.projectId && rootChat(s),
          );
          const empty = chats.length === 0;
          const open = !empty && !model.collapsed[project.projectId];
          return h(
            "section",
            { key: project.projectId, class: "project" },
            h(
              "div",
              dragProps(
                model,
                { kind: "project", id: project.projectId },
                chats.some((s) => !s.status.archived)
                  ? "project-head"
                  : "project-head dim",
              ),
              h(
                "button",
                {
                  class: "quiet project-title",
                  "data-toggle-project": project.projectId,
                  title: empty
                    ? "No chats yet; it opens when it has one. Add a chat with +. Hold to rename, or press F2"
                    : "Hold to rename, or press F2",
                  ...(empty ? { "aria-disabled": "true" } : {}),
                  "aria-keyshortcuts": "F2",
                  "aria-expanded": String(open),
                  "aria-label": `${open ? "Collapse" : "Expand"} ${project.name}`,
                  onClick: on.clicked({
                    type: "toggleProject",
                    projectId: project.projectId,
                  }),
                },
                h(
                  "svg",
                  {
                    class: "project-caret",
                    width: 14,
                    height: 14,
                    viewBox: "0 0 16 16",
                    fill: "none",
                    stroke: "currentColor",
                    "stroke-width": 1.8,
                    "stroke-linecap": "round",
                    "stroke-linejoin": "round",
                    "aria-hidden": "true",
                  },
                  h("path", { d: open ? "m4 6 4 4 4-4" : "m6 4 4 4-4 4" }),
                ),
                h("strong", {}, project.name),
              ),
              !chats.some((s) => !s.status.archived)
                ? h(
                    "button",
                    {
                      class: "quiet",
                      "aria-label": `Remove ${project.name}`,
                      title: "Remove project",
                      "data-remove-project": project.projectId,
                      onClick: on.clicked({
                        type: "requestRemoveProject",
                        projectId: project.projectId,
                      }),
                    },
                    trashIcon(),
                  )
                : null,
              h(
                "button",
                {
                  "data-new-chat": project.projectId,
                  "aria-label": `New chat in ${project.name}`,
                  title: "New chat",
                  onClick: on.clicked({
                    type: "openNewChat",
                    projectId: project.projectId,
                  }),
                },
                "+",
              ),
            ),
            open
              ? h(
                  "nav",
                  { "aria-label": `Chats in ${project.name}` },
                  chats.map((s) => {
                    const subagents = s.subagents;
                    const alerts = subagents
                      ? subagents.questions +
                        subagents.failures +
                        subagents.reportFailures
                      : 0;
                    const statusBits = [
                      !s.status.archived && s.status.queued
                        ? `${s.status.queued} ${queueWord(s.status)}`
                        : "",
                      subagents && subagents.open
                        ? `${subagents.open} subagent${subagents.open === 1 ? "" : "s"}`
                        : "",
                      alerts ? "!" : "",
                    ].filter(Boolean);
                    const statusTitle = [
                      !s.status.archived && s.status.queued
                        ? `${s.status.queued} ${queueWord(s.status)} message${s.status.queued === 1 ? "" : "s"}`
                        : "",
                      subagents && subagents.open
                        ? `${subagents.open} open subagent${subagents.open === 1 ? "" : "s"}`
                        : "",
                      subagents && subagents.questions
                        ? `${subagents.questions} open subagent question${subagents.questions === 1 ? "" : "s"}`
                        : "",
                      subagents && subagents.failures
                        ? `${subagents.failures} failed subagent turn${subagents.failures === 1 ? "" : "s"}`
                        : "",
                      subagents && subagents.reportFailures
                        ? `${subagents.reportFailures} failed subagent report${subagents.reportFailures === 1 ? "" : "s"}`
                        : "",
                    ]
                      .filter(Boolean)
                      .join("; ");
                    return h(
                      "button",
                      {
                        key: s.sessionId,
                        ...dragProps(
                          model,
                          {
                            kind: "chat",
                            id: s.sessionId,
                            projectId: project.projectId,
                          },
                          (s.sessionId === model.active
                            ? "session selected"
                            : "session") +
                            (s.status.archived ? " archived" : ""),
                        ),
                        onClick: on.clicked({
                          type: "select",
                          id: s.sessionId,
                        }),
                      },
                      !s.status.archived && s.status.turnActive
                        ? h(
                            "span",
                            {
                              class: `activity-indicator running agent-${s.agent}`,
                              role: "img",
                              "aria-label": "Running",
                            },
                            [0, 1, 2].map((i) =>
                              h("span", { key: i, "aria-hidden": "true" }),
                            ),
                          )
                        : null,
                      h(
                        "span",
                        { class: "session-title" },
                        s.name || "Untitled chat",
                      ),
                      // Every row keeps a status line so rows never change height.
                      statusBits.length
                        ? h(
                            "small",
                            {
                              class: alerts
                                ? "chat-status alert"
                                : "chat-status",
                              title: statusTitle,
                              "aria-label": statusTitle,
                            },
                            statusBits.join(" · "),
                          )
                        : h(
                            "small",
                            {
                              class: "chat-status none",
                              "aria-hidden": "true",
                            },
                            // A non-breaking space keeps the line's height with nothing shown.
                            "\u00a0",
                          ),
                      s.status.archived
                        ? h(
                            "span",
                            {
                              class: "row-delete",
                              role: "button",
                              tabindex: -1,
                              "aria-label": `Delete ${s.name || "Untitled chat"}`,
                              title: "Delete chat",
                              "data-delete-chat": s.sessionId,
                              onClick: binding((event) => {
                                event.stopPropagation();
                                return {
                                  type: "requestDeleteChat",
                                  sessionId: s.sessionId,
                                };
                              }),
                              onkeydown: binding((event) => {
                                const key = (event as KeyboardEvent).key;
                                if (key !== "Enter" && key !== " ")
                                  return { type: "noop" };
                                event.preventDefault();
                                event.stopPropagation();
                                return {
                                  type: "requestDeleteChat",
                                  sessionId: s.sessionId,
                                };
                              }),
                            },
                            trashIcon(14),
                          )
                        : numbers.has(s.sessionId)
                          ? // Every numbered chat shows its position; only 1 to 9 have a keyboard shortcut.
                            h(
                              (numbers.get(s.sessionId) ?? 10) <= 9
                                ? "kbd"
                                : "span",
                              {
                                class: `shortcut agent-${s.agent}`,
                                "aria-label": `${agentName(s.agent)}, chat ${numbers.get(s.sessionId)}`,
                                ...((numbers.get(s.sessionId) ?? 10) <= 9
                                  ? {
                                      title: `Command+${numbers.get(s.sessionId)} (Alt+${numbers.get(s.sessionId)} off a Mac)`,
                                    }
                                  : {}),
                              },
                              String(numbers.get(s.sessionId)),
                            )
                          : null,
                    );
                  }),
                )
              : null,
          );
        }),
      ),
    ),
    h(
      "section",
      { class: "chat" },
      h(
        "header",
        {},
        h(
          "button",
          {
            type: "button",
            class: "header-icon chats-toggle",
            "aria-label": "Chats",
            "aria-expanded": String(sidebarShown),
            title: "Chats",
            onClick: on.clicked({ type: "toggleSidebar" }),
          },
          h(
            "svg",
            {
              width: 20,
              height: 20,
              viewBox: "0 0 24 24",
              fill: "none",
              stroke: "currentColor",
              "stroke-width": 1.8,
              "stroke-linecap": "round",
              "aria-hidden": "true",
            },
            h("path", { d: "M4 7h16M4 12h16M4 17h16" }),
          ),
        ),
        h(
          "div",
          {},
          session
            ? h(
                "strong",
                {},
                h(
                  "button",
                  {
                    type: "button",
                    class: "chat-title",
                    "aria-label": "Rename chat",
                    title: "Rename chat",
                    onClick: on.clicked({ type: "openRename" }),
                  },
                  session.name || "Untitled chat",
                ),
              )
            : null,
          session ? h("small", {}, agentName(session.agent)) : null,
        ),
        session
          ? h(
              "button",
              {
                class: "header-icon",
                "aria-label": session.status.archived ? "Reopen" : "Archive",
                title: session.status.archived ? "Reopen" : "Archive",
                onClick: on.clicked({
                  type: "command",
                  name: session.status.archived
                    ? "reopenSession"
                    : "archiveSession",
                }),
              },
              chatActionIcon(session.status.archived ? "reopen" : "archive"),
            )
          : null,
        session?.status.archived
          ? h(
              "button",
              {
                type: "button",
                class: "header-icon",
                "aria-label": "Delete chat",
                title: "Delete chat",
                onClick: on.clicked({ type: "requestDelete" }),
              },
              trashIcon(),
            )
          : null,
        session
          ? h(
              "button",
              {
                type: "button",
                class: "header-icon chat-fullscreen",
                "aria-pressed": String(model.fullscreen),
                "aria-label": model.fullscreen
                  ? "Exit fullscreen"
                  : "Enter fullscreen",
                title: model.fullscreen
                  ? "Exit fullscreen"
                  : "Enter fullscreen",
                onClick: on.clicked({ type: "toggleFullscreen" }),
              },
              h(
                "svg",
                {
                  viewBox: "0 0 24 24",
                  width: 20,
                  height: 20,
                  fill: "none",
                  stroke: "currentColor",
                  "stroke-width": 1.75,
                  "stroke-linecap": "round",
                  "stroke-linejoin": "round",
                  "aria-hidden": "true",
                },
                h("path", {
                  d: model.fullscreen
                    ? "M3 8h5V3m8 0v5h5M3 16h5v5m8 0v-5h5"
                    : "M8 3H3v5m13-5h5v5M3 16v5h5m8 0h5v-5",
                }),
              ),
            )
          : null,
        session ? drawerToggle(drawerOpen, model.questionPendingCount) : null,
      ),
      h(
        "div",
        { class: "chat-body" },
        model.error
          ? h("p", { role: "alert", class: "error" }, model.error)
          : null,
        Object.keys(model.connection).length
          ? h(
              "p",
              { role: "status", class: "error connection" },
              `Live updates are interrupted for ${Object.keys(model.connection).join(" and ")}: ${Object.values(model.connection)[0]}. Retrying every second; what you see is what was received so far.`,
            )
          : null,
        h(
          "div",
          { class: "messages-frame" },
          h(
            "div",
            {
              id: "messages",
              class: "messages",
              tabIndex: -1,
              "aria-label": "Messages",
              "data-session-id": model.active ?? "",
              "data-history-loading": String(
                !!(model.active && model.history[model.active]?.loading),
              ),
              "data-has-older": String(
                !!(model.active && model.history[model.active]?.hasOlder),
              ),
              "data-has-newer": String(
                !!(model.active && model.history[model.active]?.hasNewer),
              ),
            },
            model.active && model.history[model.active]?.error
              ? h(
                  "div",
                  { class: "history-status", role: "alert" },
                  model.history[model.active]!.error,
                  h(
                    "button",
                    {
                      onClick: on.clicked({
                        type: "history",
                        direction: model.history[model.active]!.direction,
                      }),
                    },
                    "Try again",
                  ),
                )
              : null,
            model.active && model.history[model.active]?.loading
              ? h(
                  "p",
                  { class: "history-status", role: "status" },
                  "Loading messages…",
                )
              : null,
            model.active && model.history[model.active]?.hasOlder
              ? h(
                  "button",
                  {
                    class: "history-page",
                    disabled: model.history[model.active]?.loading,
                    onClick: on.clicked({
                      type: "history",
                      direction: "older",
                    }),
                  },
                  "Load older messages",
                )
              : null,
            session
              ? groupToolRows(rowsOf).map((row) =>
                  row.role === "marker"
                    ? h(
                        "div",
                        {
                          key: `${session.sessionId}:${row.key}`,
                          "data-row-key": row.key,
                          class: "marker",
                          role: "separator",
                        },
                        row.text,
                      )
                    : h(
                        "article",
                        {
                          key: `${session.sessionId}:${row.key}`,
                          "data-row-key": row.key,
                          class: `message ${row.role}${row.senderSessionId ? " peer" : ""}${isMessageCollapsed(model, `${session.sessionId}:${row.key}`, row.senderSessionId) ? " message-collapsed" : ""}`,
                          "data-row-keys": JSON.stringify(
                            row.sourceKeys ?? [row.key],
                          ),
                        },
                        isMessageCollapsed(
                          model,
                          `${session.sessionId}:${row.key}`,
                          row.senderSessionId,
                        )
                          ? h(
                              "div",
                              { class: "message-preview" },
                              row.text.replace(/\s+/g, " ").trim(),
                            )
                          : null,
                        messageBody(row),
                        row.attachments && row.role === "assistant"
                          ? agentImages(row, model)
                          : row.attachments
                            ? h(
                                "ul",
                                {
                                  class: "attachments sent",
                                  "aria-label": "Attachments",
                                },
                                row.attachments.map((a) =>
                                  h(
                                    "li",
                                    {
                                      key: a.id,
                                      class: "chip",
                                      "data-attachment-id": a.id,
                                    },
                                    isImage(a.contentType) && model.thumbs[a.id]
                                      ? h("img", {
                                          src: model.thumbs[a.id]!,
                                          alt: a.name,
                                        })
                                      : null,
                                    h(
                                      "button",
                                      {
                                        class: "quiet",
                                        onClick: on.clicked({
                                          type: "download",
                                          id: a.id,
                                          name: a.name,
                                        }),
                                      },
                                      a.name,
                                    ),
                                    h("small", {}, sizeText(a.size)),
                                  ),
                                ),
                              )
                            : null,
                        row.delivery === "rejected"
                          ? h(
                              "button",
                              {
                                onClick: on.clicked({
                                  type: "retryMessage",
                                  uuid: row.key,
                                }),
                                "data-retry-uuid": row.key,
                              },
                              "Retry",
                            )
                          : null,
                        row.delivery === "expired"
                          ? h(
                              "small",
                              {
                                class: "expired-note",
                                role: "note",
                                "data-expired-uuid": row.key,
                              },
                              expiredNote(row.expiredFrom),
                            )
                          : null,
                        row.text &&
                          (row.role === "user" || row.role === "assistant") &&
                          !(row.role === "assistant" && row.attachments?.length)
                          ? messageActions(row, model)
                          : null,
                      ),
                )
              : h(
                  "p",
                  { class: "empty" },
                  model.token
                    ? model.projects.length === 0
                      ? "Add a project using the folder button in the sidebar, then start a chat in it."
                      : "Select a chat, or start one with New chat on a project."
                    : "Connect with your server token.",
                ),
            model.active && model.history[model.active]?.hasNewer
              ? h(
                  "button",
                  {
                    class: "history-page",
                    disabled: model.history[model.active]?.loading,
                    onClick: on.clicked({
                      type: "history",
                      direction: "newer",
                    }),
                  },
                  "Load newer messages",
                )
              : null,
          ),
          model.newMessages ||
            (model.active &&
              (model.history[model.active]?.following === false ||
                model.history[model.active]?.hasNewer))
            ? h(
                "button",
                {
                  id: "new-messages",
                  "aria-label": "Jump to latest messages",
                  title: "Jump to latest messages",
                  disabled: !!(
                    model.active && model.history[model.active]?.loading
                  ),
                  onClick: on.clicked({ type: "bottom" }),
                },
                composerIcon("send"),
              )
            : null,
          session && model.history[model.active ?? ""]?.following !== false
            ? scrollLock()
            : null,
        ),
        ...(session
          ? pendingSecrets(model.events[session.sessionId] ?? []).map((r) =>
              secretCard(r, agentName(session.agent)),
            )
          : []),
        session
          ? h(
              "div",
              { class: "compose" },
              uncertainBanner(
                model,
                session.sessionId,
                session.agent,
                session.status.queueHeld,
              ),
              model.sendError[session.sessionId]
                ? h(
                    "div",
                    { class: "send-error", role: "alert" },
                    h("span", {}, model.sendError[session.sessionId]!),
                    h(
                      "button",
                      {
                        onClick: model.uncertain[session.sessionId]
                          ? on.clicked({ type: "retrySend" })
                          : binding(() => ({
                              type: "send",
                              id: crypto.randomUUID(),
                            })),
                      },
                      "Retry",
                    ),
                    model.uncertain[session.sessionId]
                      ? h(
                          "button",
                          { onClick: on.clicked({ type: "discardHeld" }) },
                          "Discard",
                        )
                      : null,
                  )
                : null,
              (model.attachments[session.sessionId] ?? []).length
                ? h(
                    "ul",
                    { class: "attachments", "aria-label": "Attachments" },
                    (model.attachments[session.sessionId] ?? []).map((a) =>
                      h(
                        "li",
                        {
                          key: a.id,
                          class: `chip ${a.state}`,
                          "data-attachment-id": a.id,
                        },
                        model.thumbs[a.id]
                          ? h("img", { src: model.thumbs[a.id]!, alt: "" })
                          : null,
                        h("span", { class: "chip-name" }, a.name),
                        h(
                          "small",
                          {},
                          a.state === "uploading"
                            ? `${sizeText(a.size)} · Uploading…`
                            : a.state === "failed"
                              ? `${sizeText(a.size)} · ${a.error ?? "Failed"}`
                              : sizeText(a.size),
                        ),
                        a.state === "failed"
                          ? h(
                              "button",
                              {
                                onClick: on.clicked({
                                  type: "retryUpload",
                                  id: a.id,
                                }),
                              },
                              "Retry",
                            )
                          : null,
                        h(
                          "button",
                          {
                            "aria-label": `Remove ${a.name}`,
                            onClick: on.clicked({
                              type: "removeAttachment",
                              id: a.id,
                            }),
                          },
                          "×",
                        ),
                      ),
                    ),
                  )
                : null,
              h("textarea", {
                id: "composer",
                "data-session-id": session.sessionId,
                "aria-label": "Message",
                rows: 1,
                value: "",
                readOnly: sending,
                placeholder:
                  !session.status.turnActive &&
                  !(model.drafts[session.sessionId] ?? "")
                    ? (session.status.promptSuggestion ?? "")
                    : "",
                onInput: on.inputChanged((text) => ({ type: "draft", text })),
                onpaste: binding((event) => {
                  const files = Array.from(
                    (event as ClipboardEvent).clipboardData?.files ?? [],
                  );
                  if (!files.length) return { type: "noop" };
                  event.preventDefault();
                  return {
                    type: "attach",
                    items: files.map((file) => ({
                      id: crypto.randomUUID(),
                      file,
                    })),
                  };
                }),
                onkeydown: binding((event) => {
                  const key = event as KeyboardEvent;
                  const field = event.currentTarget as HTMLTextAreaElement;
                  const suggestion = session.status.promptSuggestion;
                  if (
                    key.key === "Tab" &&
                    !key.shiftKey &&
                    !key.altKey &&
                    !key.ctrlKey &&
                    !key.metaKey &&
                    !key.isComposing &&
                    !sending &&
                    !session.status.turnActive &&
                    !field.value &&
                    suggestion
                  ) {
                    key.preventDefault();
                    field.value = suggestion;
                    field.setSelectionRange(
                      suggestion.length,
                      suggestion.length,
                    );
                    return { type: "draft", text: suggestion };
                  }
                  if (
                    key.key === "Enter" &&
                    !key.shiftKey &&
                    !key.isComposing
                  ) {
                    key.preventDefault();
                    return { type: "send", id: crypto.randomUUID() };
                  }
                  return { type: "noop" };
                }),
              }),
              h(
                "div",
                { class: "send-row toolbar" },
                h(
                  "button",
                  {
                    class: "attach icon-button",
                    "aria-label": "Attach",
                    title: "Attach files",
                    onClick: binding(() => {
                      document
                        .querySelector<HTMLInputElement>("#attach-input")
                        ?.click();
                      return { type: "noop" };
                    }),
                  },
                  composerIcon("attach"),
                ),
                h("input", {
                  id: "attach-input",
                  type: "file",
                  multiple: true,
                  hidden: true,
                  "aria-label": "Choose files to attach",
                  onChange: binding((event) => {
                    const input = event.target as HTMLInputElement;
                    const items = Array.from(input.files ?? []).map((file) => ({
                      id: crypto.randomUUID(),
                      file,
                    }));
                    input.value = "";
                    return { type: "attach", items };
                  }),
                }),
                h(
                  "div",
                  { class: "model-control" },
                  h(
                    "select",
                    {
                      "aria-label": "Model",
                      controlledValue: session.status.model,
                      onChange: binding((event) => ({
                        type: "choice",
                        model: (event.target as HTMLSelectElement).value,
                      })),
                    },
                    // A chat stays with the agent it was made for; only its models are offered.
                    // A model the catalog dropped stays selectable so the chat keeps showing it.
                    [
                      ...model.models.filter(
                        (option) => option.agent === session.agent,
                      ),
                      ...(model.models.some(
                        (option) => option.model === session.status.model,
                      )
                        ? []
                        : [
                            {
                              model: session.status.model,
                              label: session.status.model,
                            },
                          ]),
                    ].map((option) =>
                      h("option", { value: option.model }, option.label),
                    ),
                  ),
                ),
                h(
                  "div",
                  { class: "effort-control" },
                  h(
                    "select",
                    {
                      "aria-label": "Effort",
                      controlledValue: session.status.effort ?? "",
                      onChange: binding((event) => ({
                        type: "choice",
                        effort:
                          (event.target as HTMLSelectElement).value || null,
                      })),
                    },
                    h("option", { value: "" }, "Default"),
                    (
                      model.models.find((m) => m.model === session.status.model)
                        ?.efforts ?? []
                    ).map((effort) => h("option", { value: effort }, effort)),
                  ),
                ),
                session.status.selectionPending
                  ? h(
                      "small",
                      { "data-testid": "pending-selection" },
                      "Applies next turn",
                    )
                  : null,
                session.status.queued &&
                  session.status.queueHeld &&
                  session.status.heldReason
                  ? h(
                      "small",
                      {
                        class: "held-reason",
                        role: "status",
                        "data-testid": "held-reason",
                      },
                      session.status.heldReason,
                    )
                  : null,
                session.status.queued && session.status.queueHeld
                  ? h(
                      "button",
                      {
                        onClick: on.clicked({
                          type: "command",
                          name: "resumeSession",
                        }),
                      },
                      "Resume",
                    )
                  : null,
                usageMeters(session, {
                  onClick: binding(() => ({
                    type: "compact",
                    sessionId: session.sessionId,
                    id: crypto.randomUUID(),
                  })),
                }),
                h(
                  "button",
                  {
                    class: "stop icon-button",
                    "aria-label": "Stop",
                    title: "Stop (Esc)",
                    "aria-keyshortcuts": "Escape",
                    disabled:
                      session.status.stopping ||
                      !(
                        session.status.turnActive ||
                        session.status.dispatching > 0 ||
                        (session.status.queued > 0 && !session.status.queueHeld)
                      ),
                    onClick: on.clicked({
                      type: "command",
                      name: "stopSession",
                    }),
                  },
                  composerIcon("stop"),
                ),
                h(
                  "button",
                  {
                    onClick: binding(() => ({
                      type: "send",
                      id: crypto.randomUUID(),
                    })),
                    onmousedown: binding((event) => {
                      event.preventDefault();
                      return { type: "noop" };
                    }),
                    "data-send": true,
                    class: "icon-button",
                    "aria-label": sending ? "Sending" : "Send",
                    "aria-busy": sending,
                    title: "Send",
                  },
                  composerIcon("send"),
                ),
              ),
            )
          : null,
      ),
    ),
    drawerOpen
      ? h(
          "aside",
          {
            id: "chat-drawer",
            class: `subagents${model.drawerTab === "questions" ? " questions" : ""}`,
            "aria-label": "Chat details",
          },
          drawerTabs(
            model,
            subagents.filter(
              (s) => s.state === "active" || s.state === "waiting",
            ).length,
          ),
          h(
            "div",
            {
              role: "tabpanel",
              id: `drawer-panel-${model.drawerTab}`,
              "aria-labelledby": `drawer-tab-${model.drawerTab}`,
              class: "drawer-panel",
            },
            model.drawerTab === "questions"
              ? questionsContent(model)
              : model.drawerTab === "jobs"
                ? jobsContent(model)
                : model.subagentConversation
                  ? subagentConversationContent(model)
                  : agentsContent(subagents, model),
          ),
        )
      : null,
    model.lightbox && model.thumbs[model.lightbox.id]
      ? lightboxView(model, model.lightbox, model.thumbs[model.lightbox.id]!)
      : null,
    model.newChat ? newChatDialog(model, model.newChat) : null,
    model.renameChat ? renameChatDialog(model.renameChat) : null,
    model.directoryPicker ? directoryDialog(model.directoryPicker) : null,
  );
}
function directoryDialog(
  picker: NonNullable<Model["directoryPicker"]>,
): VNode<Model> {
  const listing = picker.listing;
  return h(
    "div",
    {
      class: "backdrop",
      onClick: binding((event) =>
        event.target === event.currentTarget
          ? { type: "closeDirectories" }
          : { type: "noop" },
      ),
    },
    h(
      "section",
      {
        class: "dialog directory-picker",
        role: "dialog",
        "aria-modal": "true",
        "aria-label": "Choose project folder",
        onkeydown: binding((event) => {
          const key = event as KeyboardEvent;
          if (key.key === "Escape") return { type: "closeDirectories" };
          if (key.key === "Tab") {
            const buttons = [
              ...(
                event.currentTarget as Element
              ).querySelectorAll<HTMLButtonElement>("button:not(:disabled)"),
            ];
            const first = buttons[0];
            const last = buttons.at(-1);
            if (key.shiftKey && event.target === first) {
              event.preventDefault();
              last?.focus();
            } else if (!key.shiftKey && event.target === last) {
              event.preventDefault();
              first?.focus();
            }
          }
          return { type: "noop" };
        }),
      },
      h("strong", {}, "Choose project folder"),
      h("div", { class: "directory-path" }, listing?.path ?? "Home"),
      picker.error ? h("p", { role: "alert" }, picker.error) : null,
      picker.loading ? h("p", { role: "status" }, "Loading folders…") : null,
      listing
        ? h(
            "nav",
            { class: "directory-list", "aria-label": "Folders" },
            h(
              "button",
              {
                type: "button",
                "aria-label": "Up one folder",
                disabled: !listing.parent || picker.loading,
                onClick: on.clicked(
                  listing.parent
                    ? { type: "browseDirectory", path: listing.parent }
                    : { type: "noop" },
                ),
              },
              "[..]",
            ),
            listing.directories.map((folder) =>
              h(
                "button",
                {
                  key: folder.name,
                  type: "button",
                  disabled: picker.loading,
                  onClick: on.clicked({
                    type: "browseDirectory",
                    path: folder.path,
                  }),
                },
                folder.name,
              ),
            ),
            !picker.loading && !listing.directories.length
              ? h("p", {}, "No subfolders.")
              : null,
          )
        : null,
      h(
        "div",
        { class: "dialog-actions" },
        h(
          "button",
          {
            type: "button",
            class: "quiet",
            onClick: on.clicked({ type: "closeDirectories" }),
          },
          "Cancel",
        ),
        h(
          "button",
          {
            type: "button",
            disabled: !listing || picker.loading,
            onClick: binding(() => ({
              type: "chooseDirectory",
              id: crypto.randomUUID(),
            })),
          },
          "Use this folder",
        ),
      ),
    ),
  );
}
/** Messages the agent never confirmed, listed with what the owner can do about each. */
function uncertainBanner(
  model: Model,
  sessionId: string,
  agent: string,
  held: boolean,
) {
  const rows =
    model.history[sessionId]?.timeline.rows ??
    chatRows(model.events[sessionId] ?? [], agent);
  const lost = rows.filter(
    (r) =>
      r.role === "user" &&
      r.delivery === "uncertain" &&
      !model.dismissed[r.key],
  );
  if (!lost.length) return null;
  return h(
    "div",
    { class: "uncertain-banner", role: "status" },
    h(
      "p",
      {},
      held
        ? "This chat is paused because the agent never confirmed these messages. They may or may not have reached it, and BobLM will not send them again on its own."
        : "The agent never confirmed these messages. They may or may not have reached it, and BobLM will not send them again on its own.",
    ),
    h(
      "ul",
      {},
      lost.map((row) => {
        const first = row.text.split("\n")[0]!;
        return h(
          "li",
          { key: row.key, "data-uncertain-uuid": row.key },
          h("span", {}, first.length > 120 ? `${first.slice(0, 120)}…` : first),
          h(
            "button",
            {
              onClick: on.clicked({
                type: "sendAgain",
                uuid: row.key,
                text: row.text,
              }),
            },
            "Send again",
          ),
          h(
            "button",
            {
              onClick: on.clicked({ type: "dismissUncertain", uuid: row.key }),
            },
            "Dismiss",
          ),
        );
      }),
    ),
  );
}

function secretCard(r: SecretRequest, agent: string): VNode<Model> {
  return h(
    "form",
    {
      key: r.requestId,
      class: "secret",
      "data-secret-request": r.requestId,
      onSubmit: binding((event) => {
        event.preventDefault();
        const form = event.target as HTMLFormElement;
        const input = form.elements.namedItem("secret") as HTMLInputElement;
        const value = input.value;
        // Gone from the page the moment it is sent.
        form.reset();
        return { type: "submitSecret", requestId: r.requestId, value };
      }),
    },
    h("strong", {}, `${agent} is asking for a credential`),
    h(
      "small",
      {},
      `It will be saved at ${r.key.join(".")} in ${r.file}, encrypted by SOPS. ${agent} never sees it.`,
    ),
    h("input", {
      name: "secret",
      type: "password",
      autocomplete: "off",
      "aria-label": `Value for ${r.key.join(".")}`,
      placeholder: "Paste the credential here",
      required: true,
    }),
    h(
      "div",
      { class: "dialog-actions" },
      h(
        "button",
        {
          type: "button",
          class: "quiet",
          "data-decline-secret": r.requestId,
          onClick: on.clicked({
            type: "declineSecret",
            requestId: r.requestId,
          }),
        },
        "Decline",
      ),
      h("button", { type: "submit" }, "Save"),
    ),
  );
}
const agentName = (agent: string) =>
  agent === "pi" ? "Pi" : agent === "codex" ? "Codex" : "Claude";
function forgetChat(
  model: Model,
  id: string,
): readonly [Model, ReadonlyArray<Effect>] {
  const next: Model = {
    ...model,
    sessions: model.sessions.filter((s) => s.sessionId !== id),
    active: model.active === id ? null : model.active,
    forgotten: { ...model.forgotten, [id]: true },
    subagents: model.active === id ? [] : model.subagents,
    subagentConversation:
      model.active === id ? null : model.subagentConversation,
    renameChat: model.renameChat?.sessionId === id ? null : model.renameChat,
    orderGeneration: model.orderGeneration + 1,
    drafts: without(model.drafts, id),
    pending: without(model.pending, id),
    uncertain: without(model.uncertain, id),
    sendError: without(model.sendError, id),
    events: without(model.events, id),
    history: without(model.history, id),
    cursors: without(model.cursors, id),
    attachments: without(model.attachments, id),
    connection: without(model.connection, id),
  };
  return pruneThumbnails(next, [
    { type: "persist", token: next.token, drafts: next.drafts },
    persistUncertain(next),
    { type: "persistAttachments", attachments: next.attachments },
    ...(model.active === id
      ? [{ type: "persistLastChat" as const, id: null }]
      : []),
  ]);
}
function renameChatDialog(
  chat: NonNullable<Model["renameChat"]>,
): VNode<Model> {
  return h(
    "div",
    {
      class: "backdrop",
      onClick: binding((event) =>
        (event.target as HTMLElement).classList.contains("backdrop")
          ? { type: "closeRename" }
          : { type: "noop" },
      ),
    },
    h(
      "form",
      {
        role: "dialog",
        "aria-modal": "true",
        "aria-label": "Rename chat",
        class: "dialog",
        onkeydown: binding((event) =>
          (event as KeyboardEvent).key === "Escape"
            ? { type: "closeRename" }
            : { type: "noop" },
        ),
        onSubmit: binding((event) => {
          event.preventDefault();
          return { type: "saveRename" };
        }),
      },
      h("strong", {}, "Rename chat"),
      h("input", {
        "aria-label": "Chat name",
        placeholder: "Name of chat...",
        value: chat.name,
        maxLength: limits.maxChatNameChars,
        readOnly: chat.saving,
        onInput: binding((event) => ({
          type: "renameName",
          name: (event.target as HTMLInputElement).value,
        })),
      }),
      chat.error ? h("p", { role: "alert", class: "error" }, chat.error) : null,
      h(
        "div",
        { class: "dialog-actions" },
        h(
          "button",
          {
            type: "button",
            disabled: chat.saving,
            class: "quiet",
            onClick: on.clicked({ type: "closeRename" }),
          },
          "Cancel",
        ),
        h(
          "button",
          { type: "submit", disabled: chat.saving || !chat.name.trim() },
          chat.saving ? "Saving..." : "Save",
        ),
      ),
    ),
  );
}
function newChatDialog(
  model: Model,
  chat: NonNullable<Model["newChat"]>,
): VNode<Model> {
  const project = model.projects.find((p) => p.projectId === chat.projectId);
  const efforts =
    model.models.find((m) => m.model === chat.model)?.efforts ?? [];
  const agents = [...new Set(model.models.map((m) => m.agent))];
  return h(
    "div",
    {
      class: "backdrop",
      onClick: binding((event) =>
        (event.target as HTMLElement).classList.contains("backdrop")
          ? { type: "closeNewChat" }
          : { type: "noop" },
      ),
    },
    h(
      "form",
      {
        role: "dialog",
        "aria-modal": "true",
        "aria-label": "New chat",
        class: "dialog",
        onkeydown: binding((event) =>
          (event as KeyboardEvent).key === "Escape"
            ? { type: "closeNewChat" }
            : { type: "noop" },
        ),
        onSubmit: binding((event) => {
          event.preventDefault();
          return { type: "createChat", id: crypto.randomUUID() };
        }),
      },
      h("strong", {}, "New chat"),
      h(
        "small",
        {},
        project ? `in ${project.name} (${project.path})` : "in a project",
      ),
      h("input", {
        "aria-label": "Chat name",
        placeholder: "Name of chat...",
        value: chat.name,
        maxLength: limits.maxChatNameChars,
        onInput: binding((event) => ({
          type: "newChatName",
          name: (event.target as HTMLInputElement).value,
        })),
      }),
      h(
        "div",
        { class: "new-chat-choices" },
        h(
          "div",
          { class: "new-chat-choice" },
          h(
            "select",
            {
              "aria-label": "Model",
              controlledValue: chat.model,
              onChange: binding((event) => ({
                type: "newChatModel",
                model: (event.target as HTMLSelectElement).value,
              })),
            },
            agents.map((agent) =>
              h(
                "optgroup",
                { key: agent, label: agentName(agent) },
                model.models
                  .filter((m) => m.agent === agent)
                  .map((m) => h("option", { value: m.model }, m.label)),
              ),
            ),
          ),
        ),
        h(
          "div",
          { class: "new-chat-choice" },
          h(
            "select",
            {
              "aria-label": "Reasoning effort",
              controlledValue: chat.effort ?? "",
              onChange: binding((event) => ({
                type: "newChatEffort",
                effort: (event.target as HTMLSelectElement).value || null,
              })),
            },
            h("option", { value: "" }, "Default"),
            efforts.map((effort) => h("option", { value: effort }, effort)),
          ),
        ),
      ),
      h(
        "div",
        { class: "dialog-actions" },
        h(
          "button",
          {
            type: "button",
            class: "quiet",
            onClick: on.clicked({ type: "closeNewChat" }),
          },
          "Cancel",
        ),
        h("button", { type: "submit" }, "Create"),
      ),
    ),
  );
}
export const program = defineProgram<Model, Model, Msg, Effect>({
  init: (flags) => {
    if (!flags.token) return [flags, []];
    // With the remembered list the last chat can open at once, before the server has answered.
    const wanted = flags.restoreChat;
    if (wanted && flags.sessions.some((s) => s.sessionId === wanted)) {
      const [opened, more] = step(
        { ...flags, restoreChat: null },
        { type: "select", id: wanted },
      );
      const [sent, request] = sendList(opened);
      return [sent, [request, ...more]];
    }
    const [sent, request] = sendList(flags);
    return [sent, [request]];
  },
  update,
  view,
});
type StreamRecord = {
  readonly event: string;
  readonly id: string;
  readonly data: string;
};
/** The kinds this bundle knows for a generated event union. */
function knownKinds(definition: string): ReadonlySet<string> {
  const defs = contractSchema.$defs as unknown as Record<
    string,
    { oneOf: ReadonlyArray<{ properties: { kind: { const: string } } }> }
  >;
  return new Set(defs[definition]!.oneOf.map((b) => b.properties.kind.const));
}
const LIST_KINDS = knownKinds("SessionListEvent");
const CHAT_KINDS = knownKinds("StoredEvent");
/**
 * The sequence of a well-formed event envelope whose kind this bundle does not know, so a
 * newer server can add event kinds without stalling an older tab. Null when the kind is known
 * (or the envelope is malformed), leaving the strict decoder to accept or refuse it.
 */
function unknownKindSeq(
  value: unknown,
  kinds: ReadonlySet<string>,
  record: StreamRecord,
): number | null {
  if (typeof value !== "object" || value === null) return null;
  const { kind, seq } = value as { kind?: unknown; seq?: unknown };
  if (typeof kind !== "string" || kinds.has(kind)) return null;
  if (typeof seq !== "number" || !/^\d+$/.test(record.id)) return null;
  return Number(record.id) === seq ? seq : null;
}
/** Decode one chat-list stream record; an unknown event kind only advances the cursor. */
export function decodeListRecord(record: StreamRecord): Msg {
  const raw: unknown = JSON.parse(record.data);
  const skipped = unknownKindSeq(raw, LIST_KINDS, record);
  if (record.event === "sessionListEvent" && skipped !== null)
    return { type: "unknownListEvent", seq: skipped };
  const event = parseSessionListEvent(raw);
  if (
    record.event !== "sessionListEvent" ||
    !/^\d+$/.test(record.id) ||
    parseSequence(Number(record.id)) !== event.seq
  )
    throw new Error(
      `a chat-list update arrived out of step (stream position ${JSON.stringify(record.id)} against sequence ${event.seq}) and was refused so nothing is skipped`,
    );
  return { type: "listEvent", event };
}
/** Decode one chat or subagent stream record; an unknown event kind only advances the cursor. */
export function decodeChatRecord(
  record: StreamRecord,
  sessionId: string,
  what: string,
): Msg {
  const raw: unknown = JSON.parse(record.data);
  const skipped = unknownKindSeq(raw, CHAT_KINDS, record);
  if (
    record.event === "sessionEvent" &&
    skipped !== null &&
    (raw as { sessionId?: unknown }).sessionId === sessionId
  )
    return { type: "unknownEvent", sessionId, seq: skipped };
  const event = parseStoredEvent(raw);
  if (
    record.event !== "sessionEvent" ||
    !/^\d+$/.test(record.id) ||
    parseSequence(Number(record.id)) !== event.seq ||
    event.sessionId !== sessionId
  )
    throw new Error(
      `an update for ${what} arrived out of step (stream position ${JSON.stringify(record.id)}, sequence ${event.seq}, chat ${JSON.stringify(event.sessionId)}) and was refused so nothing is skipped`,
    );
  return { type: "event", event };
}
/** The live streams, in fixed slots: the list, then the open chat (an empty slot when none). */
export function subscriptions(
  model: Model,
): ReadonlyArray<ProgramSubscription<Msg>> {
  if (!model.token || model.connectionPending)
    return [
      emptySubscription<Msg>("slot:list"),
      emptySubscription<Msg>("slot:chat-stream"),
      emptySubscription<Msg>("slot:usage-reset"),
      emptySubscription<Msg>("slot:subagent-stream"),
    ];
  const token = model.token;
  // Establish a snapshot before opening the list stream. Otherwise a reload
  // replays every historical working/idle state across every chat.
  const list =
    model.listGeneration === 0 || model.hiddenSince !== null
      ? emptySubscription<Msg>("slot:list")
      : chatStream<Msg>({
          key: `list:/sessions/events:${model.listGeneration}`,
          endpoint: "/sessions/events",
          token,
          after: String(model.listCursor),
          health: (health) => ({
            type: "connection",
            stream: "the chat list",
            health,
          }),
          decode: decodeListRecord,
        });
  const id = model.active;
  if (
    model.hiddenSince !== null ||
    !id ||
    (model.history[id] && !model.history[id].loaded)
  )
    return [
      list,
      emptySubscription<Msg>("slot:chat-stream"),
      emptySubscription<Msg>("slot:usage-reset"),
      emptySubscription<Msg>("slot:subagent-stream"),
    ];
  const usageReset = model.sessions.find((s) => s.sessionId === id);
  const resetSubscription = usageReset
    ? usageResetSubscription<Msg>(usageReset, { type: "usageExpired" })
    : null;
  const view = model.subagentConversation;
  const subagentId =
    view?.boblm === true && view.owner === id && model.history[view.key]?.loaded
      ? view.key
      : null;
  const subagentStream = subagentId
    ? chatStream<Msg>({
        key: `subagent-session:${subagentId}:${view!.generation}`,
        endpoint: `/sessions/${encodeURIComponent(subagentId)}/events`,
        token,
        after: String(model.cursors[subagentId] ?? 0),
        health: (health) => ({
          type: "connection",
          stream: "this subagent",
          health,
        }),
        decode: (record) =>
          decodeChatRecord(record, subagentId, "this subagent"),
      })
    : emptySubscription<Msg>("slot:subagent-stream");
  return [
    list,
    chatStream<Msg>({
      key: `session:${id}`,
      endpoint: `/sessions/${encodeURIComponent(id)}/events`,
      token,
      after: String(model.cursors[id] ?? 0),
      health: (health) => ({ type: "connection", stream: "this chat", health }),
      decode: (record) => decodeChatRecord(record, id, "this chat"),
    }),
    resetSubscription ?? emptySubscription<Msg>("slot:usage-reset"),
    subagentStream,
  ];
}
