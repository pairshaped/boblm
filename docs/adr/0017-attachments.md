# 0017: Attachments

The composer attaches files of any type, including pasted images. Files live on
the host, and the agent receives their paths in the message text and reads them
with its own tools. No bytes pass through the agent protocol.

## Upload and storage

A file is uploaded before the message that uses it: `PUT /sessions/{sessionId}/uploads/{uploadId}`
with the raw bytes as the body, the bearer token, `Content-Type`, and the file
name in `X-Filename` (percent-encoded UTF-8). The uploadId is a client-generated
UUID (canonical form, ADR 0004), so a retry from a flaky connection is safe: the
same id with identical bytes, name and type returns the existing record, and the
same id with anything different fails with a conflict. The body is streamed to a
temporary file while its SHA-256 and size are computed, then renamed into place.
The limit is MAX_UPLOAD_BYTES (50 MiB) per file, enforced while streaming. Uploads need an existing session. Preparing an upload leaves an archived chat
archived; sending a fresh owner prompt reopens it atomically with message acceptance.

Files are stored under `uploads/` beside the database, in a directory per session
named by the SHA-256 of the session ID, as `{uploadId}-{safe name}`. Directories
are mode 0700 and files 0600; serve refuses an existing `uploads/` directory that group
or others can use. Paths handed to the agent are absolute: relative `database` paths in
the config are made absolute when it is read. The per-session directory is naming, not
isolation: `--add-dir` covers the whole `uploads/` directory, and agents already run as
the owner (ADR 0010). The safe name keeps ASCII letters, digits, dot,
dash and underscore, replaces everything else with an underscore, drops leading
dots and is at most 100 bytes. The original name is kept in the database only for
display. Uploads are durable: they are kept for the life of the session because
the history refers to them, and nothing deletes them automatically except the
orphan sweep below.

The `attachments` table records id, session, original name, content type, size,
SHA-256, path, creation time, whether the file is stored yet and, once sent, the
message UUID. The record is written first, so a crash leaves something the sweep can
find, and the upload response, and therefore any claim by a message, waits until the
file is in place. An upload whose record exists but whose file was never stored cannot
be sent; the identical PUT completes it. The sweep deletes a file before its record,
so an interrupted sweep leaves a record to retry, never a file nothing points at. Rows and files that
are never attached to a message are deleted at server startup when older than ORPHAN_UPLOAD_AGE (24
hours), as are partial uploads (`.tmp*` files) a crash left behind. The sweep
runs before the server accepts requests. An identical retry of an upload whose
record exists but whose file is missing stores the file again.

## Sending

`sendMessage` carries `attachments`, a list of at most MAX_ATTACHMENTS (10) upload IDs (default
empty). Text may be empty when at least one attachment is present. Attachments are
linked to the message in the same transaction that appends it. An upload belongs
to one session and one message: another session's upload, an unknown ID, or an
upload already sent in a different message fails the whole send. A retry with the
same message UUID is identical only when the attachment list is identical, in which
case it returns the existing message; the usual conflict rules apply otherwise.
The message row keeps exactly the text the owner typed.

At dispatch the runner appends the attachment paths to the text:

```text
{text}

Attached files:
- /absolute/path/{uploadId}-name.png (image/png, 372 KiB, original name "Screenshot.png")
```

Steers and ordinary turns use the same text. The agent can read files outside its
working directory because sessions bypass permissions; the Claude launch also adds
the uploads directory with `--add-dir`.

## Serving

`GET /sessions/{sessionId}/uploads/{uploadId}` returns the file to an authorized
caller. The app origin holds the owner's token, so served files must not run as
scripts: every response has `X-Content-Type-Options: nosniff` and a restrictive
`Content-Security-Policy: sandbox`. PNG, JPEG, GIF and WebP are served inline with
their stored type; everything else, including SVG and HTML, is sent as
`application/octet-stream` with `Content-Disposition: attachment`.

## Browser

The composer toolbar gains an attach button (a file picker allowing several files),
and pasting an image from the clipboard attaches it. Each file uploads as soon as
it is chosen and shows as a chip with its name, size and state (uploading, ready,
failed with remove). Images show a thumbnail from the local file. Send waits until
every chip is ready; a failed chip must be removed or retried first. A sent message
shows its attachments as chips in the history; images load their thumbnail through
an authorized request. Drafts stay per device (ADR 0011), so pending uploads are
not shared between devices. Finished uploads are remembered in the browser with the
draft and return as chips after a reload; uploads still in progress do not. Removing
a chip that a held (uncertain) send names drops that held send, because a send must
not change under its UUID; the owner is told it may still have arrived. The generated
SendMessage schema allows empty content because only the server and `validate` know
whether attachments make an empty message meaningful.

## Tests

Store tests cover atomic link-and-append, conflict rules and the orphan sweep.
Runner tests use a fake driver to check the dispatched text, steering with
attachments and idempotent retry. Server tests cover streaming limits, the idempotent
upload, name and path sanitization, authorization, cross-session rejection and the
serving headers. Browser tests cover the picker, paste, chip states and sending.
