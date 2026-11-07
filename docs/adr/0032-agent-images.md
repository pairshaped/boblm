# 0032: Images an agent shows in a chat

An agent shows the owner an image with the `show_image` MCP tool (ADR 0015): the absolute path of a
PNG, JPEG, GIF or WebP file, and an optional caption of at most MAX_IMAGE_CAPTION_CHARS (200)
characters. The tool is bound to the caller's own chat like the other peer tools; there is no
argument to choose another chat.

The server checks the file's first bytes, not its name, so a text file called `shot.png` is
refused. SVG is refused because it can carry script (ADR 0017). The file must be a regular file of at
most MAX_AGENT_IMAGE_BYTES (25 MiB). Every refusal is a plain tool error the agent can read and act on
(ADR 0021): what was wrong and what to do.

An accepted image becomes an attachment of the chat, stored as an upload is (ADR 0017): a private copy
in the chat's uploads folder, so the agent may delete or overwrite its file afterwards. It is attached
to a new assistant-role message with delivery state `started`, whose text is the caption, in one
transaction. That reuses everything an owner's attachment already has: it is listed in history by the
`message_appended` event, served only to the owner's token through `GET /sessions/{id}/uploads/{id}`
with the same safe headers, left alone by the orphan sweep because it is attached, and removed with the
chat (ADR 0030). The delivery queue never sees it: only queued owner messages are dispatched.

## Browser

An assistant message that carries image attachments is drawn as one labelled section per image: the
caption, or the file name when there is none. The section is collapsible like a tool call, open to begin
with, and at full strength, not faded like tool summaries. It shows a thumbnail (the image scaled down
by the browser, at most 420 by 260 px), fetched with the owner's token like any attachment thumbnail,
for history pages and for images that arrive live. Until the picture has loaded the section says so.

Tapping or clicking the thumbnail opens the image full screen in a dialog over the page, with its label
and a close button. Escape, a click outside the picture, or the close button closes it, and focus
returns to the thumbnail. Choosing another chat closes it.

Previous and Next buttons and the left and right arrow keys move through the viewed chat's images
in timeline order, including owner attachments. Only images with a loaded thumbnail count. Navigation
never fetches older history. The controls stop at each end and show the current position when there is
more than one image. Images that finish loading become available in their timeline position. Closing
the dialog returns focus to the thumbnail that opened it, even after moving to another image.

The Claude system prompt tells the agent the tool exists; Codex agents learn it from the tool's own
description.
