# Live check: attachments end to end

Run on Claude Code 2.1.308 with the real `boblm serve` binary, an isolated
`CLAUDE_CONFIG_DIR` under `target/live` (copied credentials, removed afterward),
Haiku at low effort, and the real agent launch path (private MCP config file,
`--add-dir` on the uploads directory).

1. Uploaded a text file (`secret note.txt`) and a 64x64 red PNG over the HTTP
   upload route, then sent one message naming both as attachments.
2. Claude received the absolute paths in the message, called `Read` on both
   files, and answered with the secret word from the text and "red" for the image.
3. A second message asked Claude to call the `boblm` MCP `list_peers` tool. Claude
   launched with `--mcp-config` pointing at the private 0600 file, called
   `mcp__boblm__list_peers`, and received the tool's reply (no other sessions).
4. The MCP config file existed (mode 0600, mode 0700 directory) while the session
   ran and was gone after SIGTERM, which also ended the Claude process.

Not covered: a browser driving the same flow, and Safari.
