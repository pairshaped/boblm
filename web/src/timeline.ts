import {
  limits,
  type AttachmentInfo,
  type StoredEvent,
} from "./generated/contract.ts";
import { cleanTerminalText } from "./terminal-text.ts";

export type ToolDetail = {
  key: string;
  label: string;
  text: string;
  name: string;
  callId?: string;
  kind: "call" | "result";
  error: boolean;
};
export type ChatRow = {
  key: string;
  role: string;
  text: string;
  seq: number;
  revision: number;
  /** When the first event for this row was stored (ms since the epoch). */
  at?: number;
  delivery?: string;
  /** For an expired message: whether it was queued or unconfirmed when it expired. */
  expiredFrom?: string;
  senderSessionId?: string;
  attachments?: ReadonlyArray<AttachmentInfo>;
  tools?: ReadonlyArray<ToolDetail>;
};
export type Timeline = {
  rows: ReadonlyArray<ChatRow>;
  partial: string | null;
  partialAt: number;
  toolNames: ReadonlyMap<string, string>;
};
export const emptyTimeline = (): Timeline => ({
  rows: [],
  partial: null,
  partialAt: 0,
  toolNames: new Map(),
});

const skillPrefix = "Base directory for this skill: ";
const compactionPrefix =
  "This session is being continued from a previous conversation that ran out of context. The summary below covers the earlier portion of the conversation.";

/** Text of one more content block added to a message's row: the same text twice, or a streamed start of it, is not repeated. */
function joinText(old: string, next: string): string {
  if (!next) return old;
  if (!old) return next;
  if (next.startsWith(old)) return next;
  if (old.includes(next)) return old;
  return `${old}\n${next}`;
}
/** A tool result with terminal escape sequences removed from its text. */
function cleanToolResult(
  block: Record<string, unknown>,
): Record<string, unknown> {
  const content = block.content;
  if (typeof content === "string")
    return { ...block, content: cleanTerminalText(content) };
  if (!Array.isArray(content)) return block;
  return {
    ...block,
    content: content.map((item) => {
      if (
        item &&
        typeof item === "object" &&
        !Array.isArray(item) &&
        item.type === "text" &&
        typeof item.text === "string"
      )
        return { ...item, text: cleanTerminalText(item.text) };
      return item;
    }),
  };
}

/** Tool details of one more content block, without repeating one already on the row. */
function mergeTools(
  old: ReadonlyArray<ToolDetail>,
  next: ReadonlyArray<ToolDetail>,
): ToolDetail[] {
  const seen = new Set(old.map((t) => t.key));
  return [...old, ...next.filter((t) => !seen.has(t.key))];
}

/** Fold only incoming events. Existing rows retain their identity unless their content changes. */
export function projectEvents(
  previous: Timeline,
  events: ReadonlyArray<StoredEvent>,
  agent = "Claude",
  snapshot?: number,
): Timeline {
  let rows = previous.rows;
  let partial = previous.partial;
  let partialAt = previous.partialAt;
  const toolNames = new Map(previous.toolNames);
  let eventAt = 0;
  const put = (input: ChatRow) => {
    const index = rows.findIndex((r) => r.key === input.key);
    // A row keeps the time of the event that first created it.
    const row = { ...input, at: rows[index]?.at ?? input.at ?? eventAt };
    if (index < 0) rows = [...rows, row];
    else if (rows[index]!.revision <= row.revision) {
      const next = [...rows];
      next[index] = { ...row, seq: rows[index]!.seq };
      rows = next;
    }
  };
  for (const e of events) {
    eventAt = e.createdAt;
    const revision = snapshot ?? e.seq;
    if (e.kind === "message_appended") {
      put({
        key: e.payload.uuid,
        role: e.payload.role,
        text: e.payload.content,
        seq: e.seq,
        revision,
        delivery: e.payload.delivery_state,
        ...(e.payload.expired_from
          ? { expiredFrom: e.payload.expired_from }
          : {}),
        ...(e.payload.sender_session_id
          ? { senderSessionId: e.payload.sender_session_id }
          : {}),
        ...(e.payload.attachments?.length
          ? { attachments: e.payload.attachments }
          : {}),
      });
    } else if (e.kind === "delivery_changed") {
      const row = rows.find((r) => r.key === e.payload.uuid);
      if (row)
        put({
          ...row,
          revision,
          delivery: e.payload.delivery_state,
          ...(e.payload.expired_from
            ? { expiredFrom: e.payload.expired_from }
            : {}),
        });
    } else if (e.kind === "agent_session_changed" && e.payload.previous) {
      put({
        key: `marker:${e.seq}`,
        role: "marker",
        seq: e.seq,
        revision,
        text: `New conversation. Earlier messages stay here as history; ${agent} no longer sees them.`,
      });
    } else if (e.kind === "agent_event") {
      const raw = e.payload as Record<string, unknown>;
      if (!raw || typeof raw !== "object" || Array.isArray(raw)) continue;
      if (
        raw.type === "system" &&
        ["compact_started", "compact_boundary", "compact_interrupted"].includes(
          String(raw.subtype),
        )
      ) {
        put({
          key:
            typeof raw.compaction_id === "string"
              ? `compaction:${raw.compaction_id}`
              : `marker:${e.seq}`,
          role: "marker",
          seq: e.seq,
          revision,
          text:
            raw.subtype === "compact_started"
              ? "Compaction started. Summarizing earlier messages..."
              : raw.subtype === "compact_interrupted"
                ? "Compaction ended without confirmation. Check the agent response before retrying."
                : `Conversation compacted. ${agent} keeps a summary of the earlier messages.`,
        });
      }
      if (raw.type === "stream_event") {
        const event = raw.event as Record<string, unknown> | undefined;
        if (!event || typeof event !== "object" || Array.isArray(event))
          continue;
        if (event.type === "message_start") {
          const message = event.message as { id?: string } | undefined;
          partial =
            typeof message?.id === "string" ? message.id : `partial:${e.seq}`;
          partialAt = revision;
          put({
            key: partial,
            role: "assistant",
            text: "",
            seq: e.seq,
            revision,
          });
        } else if (event.type === "content_block_delta" && partial) {
          const row = rows.find((r) => r.key === partial);
          const delta = event.delta as { text?: unknown } | undefined;
          if (row && revision > row.revision && typeof delta?.text === "string")
            put({ ...row, revision, text: row.text + delta.text });
        }
      }
      if (raw.type === "assistant" || raw.type === "user") {
        const message = raw.message as
          { id?: string; content?: unknown } | undefined;
        if (!message || typeof message !== "object") continue;
        const tools: NonNullable<ChatRow["tools"]>[number][] = [];
        const content =
          typeof message.content === "string"
            ? [{ type: "text", text: message.content }]
            : message.content;
        const blocks = Array.isArray(content)
          ? content.filter((block, index) => {
              if (
                block &&
                typeof block === "object" &&
                (block.type === "thinking" ||
                  block.type === "redacted_thinking")
              )
                return false;
              // Claude's continuation summary is provider context, like injected skill text.
              if (
                agent === "Claude" &&
                raw.type === "user" &&
                block &&
                typeof block === "object" &&
                block.type === "text" &&
                typeof block.text === "string" &&
                block.text.startsWith(compactionPrefix)
              ) {
                tools.push({
                  key: `compaction:${index}`,
                  name: "CompactionSummary",
                  kind: "result",
                  error: false,
                  label: "Compaction summary",
                  text: block.text,
                });
                return false;
              }
              // Claude Code injects a skill's whole document as a user message right after the
              // Skill call. It opens with a fixed line, and is kept as a collapsed detail.
              if (
                raw.type === "user" &&
                block &&
                typeof block === "object" &&
                block.type === "text" &&
                typeof block.text === "string" &&
                block.text.startsWith(skillPrefix)
              ) {
                const skill =
                  block.text
                    .slice(skillPrefix.length)
                    .split("\n")[0]!
                    .trim()
                    .split("/")
                    .filter(Boolean)
                    .at(-1) ?? "skill";
                tools.push({
                  key: `skill:${index}`,
                  name: "Skill",
                  kind: "result",
                  error: false,
                  label: `Skill instructions: ${skill}`,
                  text: block.text,
                });
                return false;
              }
              if (
                !block ||
                typeof block !== "object" ||
                (block.type !== "tool_use" && block.type !== "tool_result")
              )
                return true;
              const call = block.type === "tool_use";
              const name = call
                ? typeof block.name === "string"
                  ? block.name
                  : "Tool"
                : typeof block.tool_name === "string"
                  ? block.tool_name
                  : (toolNames.get(block.tool_use_id) ?? "Tool");
              if (call && typeof block.id === "string")
                toolNames.set(block.id, name);
              const callId =
                typeof (call ? block.id : block.tool_use_id) === "string"
                  ? call
                    ? block.id
                    : block.tool_use_id
                  : undefined;
              tools.push({
                // The call id keeps keys distinct when several events add tools to one message.
                key: `${block.type}:${callId ?? index}`,
                name,
                callId,
                kind: call ? "call" : "result",
                error: !call && block.is_error === true,
                label: `${name}${call ? " call" : block.is_error ? " error" : " result"}`,
                text: JSON.stringify(
                  call
                    ? block
                    : cleanToolResult(block as Record<string, unknown>),
                  null,
                  2,
                ),
              });
              return false;
            })
          : message.content;
        const text = Array.isArray(blocks)
          ? blocks
              .map((block) =>
                block && typeof block === "object" && "text" in block
                  ? String(block.text)
                  : JSON.stringify(block, null, 2),
              )
              .join("\n")
          : typeof blocks === "string"
            ? blocks
            : JSON.stringify(message, null, 2);
        const key =
          typeof message.id === "string" ? message.id : `event:${e.seq}`;
        // Claude Code sends one event per content block (thinking, text, each tool call), all under
        // one message id. A later block adds to the row; it must not replace what came before.
        const before =
          raw.type === "assistant"
            ? rows.find((r) => r.key === key)
            : undefined;
        // Text still being streamed is only a preview of the block; its final text replaces it.
        const mergedText = !before
          ? text
          : partial === key && text
            ? text
            : joinText(before.text, text);
        const mergedTools = before
          ? mergeTools(before.tools ?? [], tools)
          : tools;
        put({
          key,
          role: raw.type === "user" ? "tool" : "assistant",
          text: mergedText,
          seq: e.seq,
          revision,
          ...(mergedTools.length ? { tools: mergedTools } : {}),
        });
        if (revision >= partialAt) {
          if (raw.history_streaming === true) {
            partial = key;
            partialAt = revision;
          } else if (key === partial) {
            partial = null;
            partialAt = revision;
          }
        }
      }
    }
  }
  while (toolNames.size > limits.historyMaxRows)
    toolNames.delete(toolNames.keys().next().value!);
  return { rows, partial, partialAt, toolNames };
}

/** Oldest-first pages can overlap live updates; keep the newest revision of each row. */
export function mergeTimeline(current: Timeline, incoming: Timeline): Timeline {
  const rows = new Map(current.rows.map((row) => [row.key, row]));
  for (const row of incoming.rows) {
    const existing = rows.get(row.key);
    if (!existing || existing.revision <= row.revision) rows.set(row.key, row);
  }
  return {
    ...current,
    rows: [...rows.values()].sort((a, b) => a.seq - b.seq),
    partial: current.partial,
    partialAt: current.partialAt,
  };
}
