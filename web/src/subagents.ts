import { emptyTimeline, projectEvents, type ChatRow } from "./timeline.ts";
import type { SessionInfo, StoredEvent } from "./generated/contract.ts";

export type Subagent = {
  key: string;
  title: string;
  state: "active" | "waiting" | "done" | "failed";
  startedAt: number;
  endedAt?: number;
  revision: number;
  /** A boblm child session; legacy native entries have no session record. */
  session?: SessionInfo;
  /** Whether a boblm child is working right now, independent of its open/archived state. */
  busy?: boolean;
};

/** Claude's tool for starting a subagent (called Task in older versions). */
const claudeTools = new Set(["agent", "task"]);

const humanize = (path: string): string => {
  const slug = path.split("/").filter(Boolean).at(-1) ?? "subagent";
  const text = slug.replace(/[_-]+/g, " ").trim();
  return text ? text[0]!.toUpperCase() + text.slice(1) : "Subagent";
};

const parse = (text: string): Record<string, unknown> | null => {
  try {
    const value: unknown = JSON.parse(text);
    return value && typeof value === "object" && !Array.isArray(value)
      ? (value as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
};

/**
 * The subagents a chat has started, read from its loaded rows: active ones first, then finished
 * ones newest first. Claude's Agent calls finish with their matching result; Codex reports
 * `started` and `completed` per thread, and the parent agent itself (path /root) is not one.
 */
export function subagentsFrom(
  rows: ReadonlyArray<Pick<ChatRow, "tools" | "at" | "revision">>,
  known: ReadonlyArray<Subagent> = [],
): Subagent[] {
  const found = new Map(known.map((agent) => [agent.key, agent]));
  for (const row of rows) {
    const at = row.at ?? 0;
    for (const tool of row.tools ?? []) {
      const name = tool.name.toLowerCase();
      const knownResult =
        tool.kind === "result" && !!tool.callId && found.has(tool.callId);
      if (!claudeTools.has(name) && name !== "subagentactivity" && !knownResult)
        continue;
      const block = parse(tool.text);
      if (!block) continue;
      if (claudeTools.has(name) || knownResult) {
        if (
          tool.callId &&
          (found.get(tool.callId)?.revision ?? -1) >= row.revision
        )
          continue;
        if (tool.kind === "call" && tool.callId) {
          const input = (block.input ?? {}) as Record<string, unknown>;
          const title = [input.description, input.subagent_type].find(
            (v): v is string => typeof v === "string" && v.trim() !== "",
          );
          found.set(tool.callId, {
            key: tool.callId,
            title: title ?? "Subagent",
            state: "active",
            startedAt: at,
            revision: row.revision,
          });
        } else if (tool.kind === "result" && tool.callId) {
          const started = found.get(tool.callId);
          if (started)
            found.set(tool.callId, {
              ...started,
              state: tool.error ? "failed" : "done",
              endedAt: at,
              revision: row.revision,
            });
        }
      } else if (name === "subagentactivity" && tool.kind === "call") {
        const input = (block.input ?? {}) as Record<string, unknown>;
        const path = typeof input.agentPath === "string" ? input.agentPath : "";
        const thread =
          typeof input.agentThreadId === "string" ? input.agentThreadId : "";
        if (!thread || path === "/root" || path === "") continue;
        const known = found.get(thread);
        if ((known?.revision ?? -1) >= row.revision) continue;
        if (input.kind === "started")
          found.set(thread, {
            key: thread,
            title: humanize(path),
            state: "active",
            startedAt: at,
            revision: row.revision,
          });
        else if (input.kind === "completed")
          found.set(thread, {
            key: thread,
            title: known?.title ?? humanize(path),
            state: "done",
            startedAt: known?.startedAt ?? at,
            endedAt: at,
            revision: row.revision,
          });
      }
    }
  }
  const all = [...found.values()];
  const active = all.filter((s) => s.state === "active");
  const ended = all
    .filter((s) => s.state !== "active")
    .sort((a, b) => (b.endedAt ?? 0) - (a.endedAt ?? 0));
  return [...active.sort((a, b) => b.startedAt - a.startedAt), ...ended];
}

/** Merge the root's boblm children from the session list with any legacy native entries. */
export function subagentsFromSessions(
  sessions: ReadonlyArray<SessionInfo>,
  rootId: string,
  known: ReadonlyArray<Subagent> = [],
): Subagent[] {
  const found = new Map(known.map((agent) => [agent.key, agent]));
  for (const session of sessions) {
    if (session.parentSessionId !== rootId) continue;
    const status = session.status;
    const busy =
      status.turnActive ||
      status.stopping ||
      status.dispatching > 0 ||
      status.queued > 0;
    found.set(session.sessionId, {
      key: session.sessionId,
      title: session.name.trim() || "Subagent",
      state: status.archived ? "done" : "active",
      startedAt: session.createdAt,
      revision: Number.MAX_SAFE_INTEGER,
      session,
      busy,
    });
  }
  const all = [...found.values()];
  const active = all.filter((agent) => agent.state === "active");
  const ended = all
    .filter((agent) => agent.state !== "active")
    .sort((a, b) => (b.endedAt ?? 0) - (a.endedAt ?? 0));
  return [...active.sort((a, b) => b.startedAt - a.startedAt), ...ended];
}

/** Lifecycle state is independent of which message rows the browser currently retains. */
export function subagentsFromEvents(
  events: ReadonlyArray<StoredEvent>,
  known: ReadonlyArray<Subagent> = [],
): ReadonlyArray<Subagent> {
  const candidates = events
    .map((event) => ({ event, blocks: lifecycleBlocks(event) }))
    .filter((item) => item.blocks.length);
  if (!candidates.length) return known;
  const keys = new Set(known.map((agent) => agent.key));
  for (const { blocks } of candidates)
    for (const block of blocks)
      if (
        block.type === "tool_use" &&
        typeof block.id === "string" &&
        typeof block.name === "string" &&
        claudeTools.has(block.name.toLowerCase())
      )
        keys.add(block.id);
  const relevant = candidates.filter(({ blocks }) =>
    blocks.some(
      (block) =>
        (block.type === "tool_use" &&
          typeof block.name === "string" &&
          (claudeTools.has(block.name.toLowerCase()) ||
            block.name.toLowerCase() === "subagentactivity")) ||
        (block.type === "tool_result" &&
          typeof block.tool_use_id === "string" &&
          keys.has(block.tool_use_id)),
    ),
  );
  if (!relevant.length) return known;
  return subagentsFrom(
    relevant.flatMap(
      ({ event }) => projectEvents(emptyTimeline(), [event]).rows,
    ),
    known,
  );
}
function lifecycleBlocks(
  event: StoredEvent,
): ReadonlyArray<Record<string, unknown>> {
  if (
    event.kind !== "agent_event" ||
    !event.payload ||
    typeof event.payload !== "object" ||
    !("message" in event.payload)
  )
    return [];
  const message = event.payload.message;
  if (
    !message ||
    typeof message !== "object" ||
    !("content" in message) ||
    !Array.isArray(message.content)
  )
    return [];
  const blocks: unknown[] = message.content;
  return blocks.filter(
    (block): block is Record<string, unknown> =>
      !!block &&
      typeof block === "object" &&
      !Array.isArray(block) &&
      "type" in block &&
      (block.type === "tool_use" || block.type === "tool_result"),
  );
}
