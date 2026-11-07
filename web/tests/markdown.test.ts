import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><body></body>");
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
});
const { renderMarkdown } = await import("../src/markdown.ts");

const parse = (markdown: string) => {
  const host = dom.window.document.createElement("div");
  host.innerHTML = renderMarkdown(markdown);
  return host;
};

test("a table renders as a table inside a sideways-scrolling wrapper", () => {
  const host = parse("| a | b |\n|---|---|\n| 1 | 2 |\n");
  const table = host.querySelector(".table-scroll > table");
  assert.ok(table, "table sits in its scroll wrapper");
  assert.equal(table.querySelectorAll("th").length, 2);
  assert.equal(table.querySelectorAll("td").length, 2);
});

test("headings, lists, emphasis, inline code and blockquotes render", () => {
  const host = parse(
    "# Title\n\n- one\n- two\n\n1. first\n\n**bold** and *italic* and `code`\n\n> quoted\n",
  );
  assert.equal(host.querySelector("h1")?.textContent, "Title");
  assert.equal(host.querySelectorAll("ul li").length, 2);
  assert.equal(host.querySelectorAll("ol li").length, 1);
  assert.equal(host.querySelector("strong")?.textContent, "bold");
  assert.equal(host.querySelector("em")?.textContent, "italic");
  assert.equal(host.querySelector("p code")?.textContent, "code");
  assert.equal(host.querySelector("blockquote")?.textContent?.trim(), "quoted");
});

test("a fenced code block keeps its text exactly and gets its own copy button", () => {
  const host = parse("```js\nconst a = '<b>' && 1;\n```\n");
  const block = host.querySelector(".code-block");
  assert.ok(block);
  assert.equal(
    block.querySelector("code")?.textContent,
    "const a = '<b>' && 1;",
  );
  assert.ok(block.querySelector("code.language-js"));
  const button = block.querySelector("button.copy-code");
  assert.equal(button?.getAttribute("type"), "button");
  assert.ok(button?.getAttribute("aria-label"));
});

test("links open in a new tab without handing over the opener", () => {
  const link = parse("[site](https://example.com/x)").querySelector("a");
  assert.equal(link?.getAttribute("href"), "https://example.com/x");
  assert.equal(link?.getAttribute("target"), "_blank");
  assert.match(link?.getAttribute("rel") ?? "", /noopener/);
  assert.match(link?.getAttribute("rel") ?? "", /noreferrer/);
});

test("hostile input renders inert", () => {
  const host = parse(
    [
      "<script>window.pwned = 1</script>",
      "",
      "[click](javascript:alert(1))",
      "",
      "![x](https://tracker.example/p.png)",
      "",
      '<img src=x onerror="window.pwned = 2">',
      "",
      '<a href="javascript:alert(3)" onclick="window.pwned = 4">raw</a>',
      "",
      "<iframe src='https://evil.example'></iframe>",
      "",
      "```html\n<script>alert(5)</script>\n```",
    ].join("\n"),
  );
  assert.equal(host.querySelector("script"), null);
  assert.equal(
    host.querySelector("img"),
    null,
    "remote images are never loaded",
  );
  assert.equal(host.querySelector("iframe"), null);
  for (const a of host.querySelectorAll("a")) {
    assert.doesNotMatch(a.getAttribute("href") ?? "", /^javascript:/i);
    assert.equal(a.getAttribute("onclick"), null);
  }
  assert.equal(host.querySelector("[onerror]"), null);
  assert.equal((dom.window as unknown as { pwned?: number }).pwned, undefined);
  assert.match(
    host.textContent ?? "",
    /<script>alert\(5\)<\/script>/,
    "code stays visible as text",
  );
});

test("raw HTML in prose is shown as text, not rendered", () => {
  const host = parse('a <div class="x">boxed</div> b');
  assert.equal(host.querySelector("div.x"), null);
  assert.match(host.textContent ?? "", /<div class="x">boxed<\/div>/);
});

test("half-written markdown still renders without throwing", () => {
  assert.doesNotThrow(() => renderMarkdown("```js\nlet a = 1"));
  assert.doesNotThrow(() => renderMarkdown("| a | b |\n|---"));
  assert.doesNotThrow(() => renderMarkdown("**unclosed"));
  assert.equal(renderMarkdown(""), "");
});

test("the same text renders to the same HTML every time", () => {
  assert.equal(renderMarkdown("**x**"), renderMarkdown("**x**"));
});
