// Loaded explicitly by boblm. Discovery is completed before readiness is registered.
export default async function (pi) {
  const url = process.env.BOBLM_MCP_URL;
  const token = process.env.BOBLM_MCP_TOKEN;
  const deadline = Number(process.env.BOBLM_MCP_TIMEOUT_MS);
  const maxBytes = Number(process.env.BOBLM_MCP_MAX_BYTES);
  let sequence = 0;
  async function rpc(method, params, signal) {
    const id = ++sequence;
    const controller = AbortSignal.timeout(deadline);
    const response = await fetch(url, {
      method: "POST",
      headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json", Accept: "application/json, text/event-stream", "MCP-Protocol-Version": "2025-03-26" },
      body: JSON.stringify({ jsonrpc: "2.0", id, method, params }),
      signal: signal ? AbortSignal.any([signal, controller]) : controller,
    });
    if (!response.ok) { await response.body?.cancel(); throw new Error(`BobLM returned HTTP ${response.status}. Check the peer-tools endpoint.`); }
    let size = 0;
    const chunks = [];
    for await (const chunk of response.body) {
      size += chunk.length;
      if (size > maxBytes) throw new Error("The BobLM tool reply exceeded the response limit. Request less data.");
      chunks.push(chunk);
    }
    const reply = JSON.parse(Buffer.concat(chunks).toString("utf8"));
    if (reply.id !== id || reply.jsonrpc !== "2.0") throw new Error("BobLM returned a mismatched tool reply. Inspect the chat before retrying.");
    if (reply.error) throw new Error("BobLM refused the peer-tool request. Check its arguments before retrying.");
    return reply.result;
  }
  await rpc("initialize", { protocolVersion: "2025-03-26", capabilities: {}, clientInfo: { name: "boblm-pi", version: "1" } });
  const listing = await rpc("tools/list", {});
  if (!Array.isArray(listing.tools) || listing.tools.length === 0) throw new Error("BobLM returned no peer tools. Check the server before starting this chat.");
  for (const tool of listing.tools) {
    pi.registerTool({ name: tool.name, label: tool.name, description: tool.description ?? tool.name, parameters: tool.inputSchema,
      async execute(_id, args, signal) {
        try {
          const result = await rpc("tools/call", { name: tool.name, arguments: args }, signal);
          if (result.isError) throw new Error(result.content?.filter(c => c.type === "text").map(c => c.text).join("\n") || "BobLM refused the tool call. Check its arguments.");
          return { content: result.content, details: {} };
        } catch (error) {
          // Fetch errors never include headers; redact an accidental endpoint echo as well.
          const message = String(error?.message ?? "The BobLM tool request did not finish. Inspect the chat before retrying.");
          throw new Error(token ? message.replaceAll(token, "[redacted]") : message);
        }
      },
    });
  }
  pi.registerCommand("boblm-peer-tools-ready", { description: "Internal boblm readiness check", handler: async () => {} });
}
