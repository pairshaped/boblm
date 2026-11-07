# 0041: boblm-provided web tools

`web_search` and `web_fetch` are registered on the same in-process HTTP MCP server as
the other peer tools (ADR 0015), so every agent that connects gets them. Pi needs no
driver change: its peer-tools extension registers every tool from `tools/list`. Claude
and Codex keep their built-in search; these tools are available to all three.

## web_search

Calls OpenAI's Responses API with the hosted `web_search` tool. The model is
`gpt-6-luna` (`OPENAI_WEB_SEARCH_MODEL`), the cheapest model that both supports the
tool and is not deprecated at the time of writing: $0.10 input and $0.50 output per
million tokens, plus $10 per 1,000 search calls. Reasoning effort is `low`
(`OPENAI_WEB_SEARCH_REASONING_EFFORT`) to keep latency and cost down, and the request
sets `store: false` so OpenAI does not retain the response. It also sets
`tool_choice: "required"`, the form the Responses API documents for a search that must
run, so the model cannot answer from its own knowledge and skip the hosted search. The
endpoint (`OPENAI_WEB_SEARCH_URL`) and the model are tunables. The reply carries the model's
answer and its `url_citation` annotations, deduplicated by URL, each with its URL and
title. OpenAI is the only backend for now; Kagi is a later fallback.

## web_fetch

BobLM fetches the page itself, so no third party sees the URL. Only `http` and
`https` are allowed. The body is read with a byte cap (`WEB_FETCH_MAX_BYTES`) and the
request has a deadline (`WEB_FETCH_TIMEOUT`); a page that exceeds either is refused
with the limit named. HTML becomes readable plain text with Markdown links, a title,
and a character cap (`WEB_FETCH_MAX_CHARS`); other `text/*` types are returned as they
are. A content type that is neither HTML nor text is refused. The conversion is a
small dependency-free pass that skips script, style and other non-text elements,
decodes common entities and numeric references, and normalizes whitespace.

web_fetch refuses local and private network addresses before it connects: loopback,
unspecified, private, link-local, CGNAT (`100.64.0.0/10`, which includes Tailscale
addresses), IPv6 unique-local, multicast, broadcast, and the IPv4-mapped IPv6 forms of
those. It resolves the host itself, checks every returned address, and connects to the
checked address through reqwest's `resolve` override, so a second lookup cannot swap in
a different address. Literal IP URLs get the same check. Redirects are followed
manually, one hop at a time, with the check repeated for each hop up to
`WEB_FETCH_MAX_REDIRECTS`. The refusal names the address and its class, for example
"web_fetch does not fetch local or private network addresses; 127.0.0.1 is loopback."
Tests inject loopback access and a fixed resolution through an address policy; the
production policy always refuses loopback.

## Credentials

The OpenAI key is the single object-key path `["OPENAI_API_KEY"]` in
`~/.config/sops/user-secrets.yaml` (`OPENAI_SOPS_FILE`, `OPENAI_SOPS_KEY`), extracted
per use through the same one-key SOPS boundary the Pi driver uses (ADR 0026). The
value goes into one request header and is never logged, stored, cached in a file, or
put on a command line.

## Untrusted content

Both tools return text that came from the web or from a model that read the web. Each
reply begins with a banner telling the agent to treat it as data, never as
instructions, and the tool descriptions say the same. Page and search text is never
executed or interpreted as BobLM input.

## Errors and tests

Failures follow ADR 0021 and name the request, the provider status or reason, the
limit that was exceeded, or the credential path, with a next step.
`crates/server/tests/web_tools.rs` exercises both tools against fake local HTTP
endpoints: the answer and cited sources, a provider error, a timeout, an oversized
page, a non-text page, a bad URL scheme, an empty query, a direct loopback URL, a host name
that resolves to loopback, a redirect to a private address, and a missing key through
the real SOPS boundary. `crates/server/examples/web-probe.rs` makes one real search and
one real fetch through the MCP boundary when a live check is authorized.
