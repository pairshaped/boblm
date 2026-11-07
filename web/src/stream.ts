import type { ProgramMsg, ProgramSubscription } from "@pairshaped/hypertea";
import {
  eventStream,
  liveSubscription,
  LiveStreamError,
  type LiveHealth,
  type SseRecord,
} from "@pairshaped/hypertea/live";
import { limits } from "./generated/contract.ts";

/**
 * boblm's wording for a live stream that failed to open or broke. Hypertea supplies neutral
 * text and a structured failure; this adds the cause the owner can act on (the token, the
 * Tailscale link). Errors that did not come from the transport pass through unchanged.
 */
export function describeStreamError(error: unknown, text: string): string {
  if (!(error instanceof LiveStreamError)) return text;
  const failure = error.failure;
  switch (failure.kind) {
    case "network": {
      const reason =
        error.cause instanceof Error && error.cause.message
          ? error.cause.message
          : "network failure";
      return `could not reach the server (${reason}); check the connection and the Tailscale link`;
    }
    case "rejected":
      return `the server answered HTTP ${failure.status} when opening the live stream: the token was rejected. Lock and unlock with the token that \`boblm init\` printed`;
    case "status":
      return `the server answered HTTP ${failure.status} when opening the live stream`;
    case "content-type":
      return `the live stream answered with ${failure.contentType ?? "no content type"}, which is not an event stream (a proxy in front of the server may have answered instead)`;
    case "no-body":
      return "the server answered the live stream request but sent no stream, so there is nothing to read";
    case "record-too-large":
      return `one event from the server was larger than the ${failure.limitBytes >> 20} MiB limit, so it was refused rather than half-read`;
    case "invalid-utf8":
      return "the server sent text that is not valid UTF-8, so the live stream was dropped";
    case "disconnected":
      return "the connection to the server dropped while the live stream was open; check the connection and the Tailscale link";
    default:
      return assertNever(failure);
  }
}
function assertNever(value: never): never {
  throw new Error(`unhandled live stream failure ${JSON.stringify(value)}`);
}

export type StreamOptions<Msg extends ProgramMsg> = Readonly<{
  /** Names the stream and its resource, never its cursor. */
  key: string;
  endpoint: string;
  token: string;
  /** The last event ID this page has, to resume from. */
  after: string;
  /** Turn one event into a message, or throw to refuse it (the cursor then stays put). */
  decode: (record: SseRecord) => Msg;
  health: (health: LiveHealth) => Msg;
  onCursor?: (id: string) => void;
  onError?: (error: unknown) => void;
  fetch?: typeof fetch;
  /** Pause before reconnecting; tests shorten it. */
  retryMs?: number;
}>;

/** A server-sent event stream that resumes from the last accepted event ID. */
export function chatStream<Msg extends ProgramMsg>(
  options: StreamOptions<Msg>,
): ProgramSubscription<Msg> {
  const retryMs = options.retryMs ?? 1000;
  return liveSubscription<SseRecord, string, Msg>({
    key: options.key,
    after: options.after,
    open: eventStream<string>({
      url: options.endpoint,
      headers: { Authorization: `Bearer ${options.token}` },
      cursor: { header: "Last-Event-ID" },
      maxEventBytes: limits.maxBodyBytes,
      describeError: describeStreamError,
      ...(options.fetch ? { fetch: options.fetch } : {}),
    }),
    decode: (record) => ({
      message: options.decode(record),
      cursor: record.id,
    }),
    retry: { initialMs: retryMs, maxMs: retryMs, resetOn: "open" },
    health: options.health,
    ...(options.onCursor
      ? { onCursor: (id) => id !== undefined && options.onCursor?.(id) }
      : {}),
    ...(options.onError ? { onError: options.onError } : {}),
  });
}
