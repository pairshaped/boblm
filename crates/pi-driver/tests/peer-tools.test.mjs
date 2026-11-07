import test from "node:test";
import assert from "node:assert/strict";
import http from "node:http";
import extension from "../src/peer-tools.mjs";

test("Pi peer tools discover schemas, bind the token, and propagate failed calls", async () => {
  const methods = [];
  const server = http.createServer(async (req, res) => {
    assert.equal(req.headers.authorization, "Bearer fictional-capability");
    let data = "";
    for await (const chunk of req) data += chunk;
    const msg = JSON.parse(data);
    methods.push(msg.method);
    const result = msg.method === "initialize" ? {} : msg.method === "tools/list"
      ? { tools: [{ name: "list_projects", description: "List projects", inputSchema: { type: "object", properties: {} } }] }
      : msg.params.arguments.fail
        ? { isError: true, content: [{ type: "text", text: "Denied fictional-capability" }] }
        : { content: [{ type: "text", text: "projects" }] };
    res.setHeader("content-type", "application/json");
    res.end(JSON.stringify({ jsonrpc: "2.0", id: msg.id, result }));
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  try {
    process.env.BOBLM_MCP_URL = `http://127.0.0.1:${server.address().port}/mcp/caller`;
    process.env.BOBLM_MCP_TOKEN = "fictional-capability";
    process.env.BOBLM_MCP_TIMEOUT_MS = "1000";
    process.env.BOBLM_MCP_MAX_BYTES = "10000";
    const tools = [];
    let ready = false;
    await extension({ registerTool: t => tools.push(t), registerCommand: name => { ready = name === "boblm-peer-tools-ready"; } });
    assert.equal(ready, true);
    assert.equal(tools[0].name, "list_projects");
    assert.equal((await tools[0].execute("call", {})).content[0].text, "projects");
    await assert.rejects(tools[0].execute("call", { fail: true }), /Denied \[redacted\]/);
    assert.deepEqual(methods, ["initialize", "tools/list", "tools/call", "tools/call"]);
  } finally { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); }
});
