import { limits, parseListSessionsResponse } from "./generated/contract.ts";
import { postCommand, type CommandOutcome } from "./http.ts";
import type { ChatList } from "./app.ts";

/** Authentication uses the entered token before it becomes the saved transport token. */
export async function connectToServer(
  token: string,
  request: typeof fetch = fetch,
  timeoutMs: number = limits.browserConnectTimeoutMs,
): Promise<CommandOutcome<ChatList>> {
  const controller = new AbortController();
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<CommandOutcome<ChatList>>((resolve) => {
    timer = setTimeout(() => {
      resolve({
        kind: "uncertain",
        reason: `The connection check timed out after ${timeoutMs / 1000} seconds.`,
      });
      controller.abort();
    }, timeoutMs);
  });
  try {
    return await Promise.race([
      postCommand(
        "/commands/listSessions",
        {},
        token,
        parseListSessionsResponse,
        (input, init) => request(input, { ...init, signal: controller.signal }),
      ),
      timeout,
    ]);
  } finally {
    clearTimeout(timer);
  }
}
