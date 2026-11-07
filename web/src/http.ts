import {
  limits,
  parseErrorInfo,
  parseListSessionsResponse,
  parseDirectoryListing,
  parseChatHistoryPage,
  parseSecretReply,
  parsePushConfigReply,
  parseChatPresenceReply,
  parseQuestionsReply,
  parseSessionInfo,
  parseSendMessageResponse,
  parseCommandResponse,
  parseProjectInfo,
  parseEmpty,
  type ErrorInfo,
} from "./generated/contract.ts";
export type CommandOutcome<T> =
  | Readonly<{ kind: "replied"; reply: T }>
  | Readonly<{ kind: "rejected"; error: ErrorInfo }>
  | Readonly<{ kind: "uncertain"; reason?: string }>;

/** Decode each command's actual reply at the HTTP boundary. */
export function parseCommandReply(name: string, value: unknown): unknown {
  if (name === "chatPresence") return parseChatPresenceReply(value);
  if (name === "pushConfig") return parsePushConfigReply(value);
  if (name === "listSessions") return parseListSessionsResponse(value);
  if (name === "chatHistory") return parseChatHistoryPage(value);
  if (name === "listDirectories") return parseDirectoryListing(value);
  if (
    name === "listQuestions" ||
    name === "answerQuestion" ||
    name === "discussQuestionLater"
  )
    return parseQuestionsReply(value);
  if (name === "submitSecret") return parseSecretReply(value);
  if (name === "createSession") return parseSessionInfo(value);
  if (name === "sendMessage") return parseSendMessageResponse(value);
  if (name === "addProject" || name === "renameProject")
    return parseProjectInfo(value);
  if (
    name === "subscribePush" ||
    name === "unsubscribePush" ||
    name === "deleteSession" ||
    name === "removeProject" ||
    name === "reorderProjects" ||
    name === "reorderChats"
  )
    return parseEmpty(value);
  return parseCommandResponse(value);
}

/** Never retry a command automatically. A deliberate resend retains the original message UUID. */
export async function postCommand<T>(
  endpoint: string,
  body: unknown,
  token: string,
  parse: (value: unknown) => T,
  request: typeof fetch = fetch,
): Promise<CommandOutcome<T>> {
  return requestJson(
    endpoint,
    {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${token}`,
      },
      body: JSON.stringify(body),
    },
    parse,
    request,
  );
}

/**
 * Upload one file under a client-generated ID. The server treats a repeat of the
 * same ID and bytes as the same upload, so a deliberate retry is safe.
 */
export async function putUpload<T>(
  sessionId: string,
  uploadId: string,
  file: File,
  token: string,
  parse: (value: unknown) => T,
  request: typeof fetch = fetch,
): Promise<CommandOutcome<T>> {
  return requestJson(
    `/sessions/${encodeURIComponent(sessionId)}/uploads/${uploadId}`,
    {
      method: "PUT",
      headers: {
        "Content-Type": file.type || "application/octet-stream",
        "X-Filename": encodeURIComponent(file.name),
        Authorization: `Bearer ${token}`,
      },
      body: file,
    },
    parse,
    request,
  );
}

/** Fetch an upload with the bearer token; null when it cannot be read. */
export async function fetchUpload(
  sessionId: string,
  uploadId: string,
  token: string,
  request: typeof fetch = fetch,
): Promise<Blob | null> {
  try {
    const response = await request(
      `/sessions/${encodeURIComponent(sessionId)}/uploads/${uploadId}`,
      { headers: { Authorization: `Bearer ${token}` } },
    );
    return response.ok ? await response.blob() : null;
  } catch {
    return null;
  }
}

async function requestJson<T>(
  endpoint: string,
  init: RequestInit,
  parse: (value: unknown) => T,
  request: typeof fetch,
): Promise<CommandOutcome<T>> {
  const unsure = (reason: string): CommandOutcome<T> => ({
    kind: "uncertain",
    reason,
  });
  let response: Response;
  try {
    response = await request(endpoint, init);
  } catch (error) {
    return unsure(
      `could not reach the server (${error instanceof Error ? error.message : "network failure"}); check the connection and the Tailscale link`,
    );
  }
  const status = `HTTP ${response.status}`;
  try {
    if (!response.body)
      return unsure(`the server answered ${status} with no body`);
    const reader = response.body.getReader();
    const chunks: Uint8Array[] = [];
    let size = 0;
    try {
      while (true) {
        const chunk = await reader.read();
        if (chunk.done) break;
        size += chunk.value.byteLength;
        if (size > limits.maxBodyBytes)
          return unsure(
            `the reply was larger than the ${limits.maxBodyBytes >> 20} MiB limit (${status})`,
          );
        chunks.push(chunk.value);
      }
    } finally {
      await reader.cancel().catch(() => {});
      reader.releaseLock();
    }
    const bytes = new Uint8Array(size);
    let offset = 0;
    for (const chunk of chunks) {
      bytes.set(chunk, offset);
      offset += chunk.byteLength;
    }
    let value: unknown;
    try {
      value = JSON.parse(
        new TextDecoder("utf-8", { fatal: true }).decode(bytes),
      );
    } catch {
      return unsure(
        `the server answered ${status} with something that is not a BobLM reply (a proxy or gateway in front of it may have answered instead)`,
      );
    }
    if (!response.ok) {
      try {
        return { kind: "rejected", error: parseErrorInfo(value) };
      } catch {
        return unsure(
          `the server answered ${status} with a body that is not a BobLM error message`,
        );
      }
    }
    try {
      return { kind: "replied", reply: parse(value) };
    } catch (error) {
      return unsure(
        `the reply was not in the expected form. ${error instanceof Error ? error.message : ""}`.trim(),
      );
    }
  } catch (error) {
    return unsure(
      `the reply could not be read (${error instanceof Error ? error.message : "read failure"})`,
    );
  }
}
