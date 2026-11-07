import createDOMPurify from "dompurify";
import { Marked } from "marked";

const escapeHtml = (text: string) =>
  text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");

// Agent output is untrusted: raw HTML in the text is shown as text, and images are
// never fetched (a remote image is a tracking and exfiltration channel).
const parser = new Marked({
  gfm: true,
  renderer: {
    html: ({ text }) => escapeHtml(text),
    image: ({ text }) => escapeHtml(text),
    code: ({ text, lang }) => {
      const language = (lang ?? "").split(/\s/)[0]!.replace(/[^\w+#.-]/g, "");
      return `<div class="code-block"><button type="button" class="copy-code" aria-label="Copy code">Copy</button><pre><code${language ? ` class="language-${language}"` : ""}>${escapeHtml(text)}</code></pre></div>`;
    },
  },
});

type Purifier = ReturnType<typeof createDOMPurify>;
let purifier: { window: Window; instance: Purifier } | undefined;

/** Built on first use against the live window, so a missing window is an error, never unsanitized output. */
function sanitizer(): Purifier {
  const win = (globalThis as { window?: Window }).window;
  if (!win)
    throw new Error(
      "Markdown can only be rendered where a browser window exists to sanitize it with.",
    );
  if (purifier?.window !== win) {
    const instance = createDOMPurify(win as never);
    instance.addHook("afterSanitizeAttributes", (node) => {
      if (node.tagName === "A") {
        node.setAttribute("target", "_blank");
        node.setAttribute("rel", "noopener noreferrer");
      }
    });
    purifier = { window: win, instance };
  }
  return purifier.instance;
}

const cache = new Map<string, string>();
const cacheLimit = 400;

/** Sanitized HTML for an agent message. Rendering is pure in the text, so results are cached. */
export function renderMarkdown(text: string): string {
  const hit = cache.get(text);
  if (hit !== undefined) return hit;
  const html = sanitizer()
    .sanitize(parser.parse(text, { async: false }), {
      FORBID_TAGS: ["style", "img", "form", "input", "textarea", "select"],
      FORBID_ATTR: ["style"],
    })
    .replaceAll("<table>", '<div class="table-scroll"><table>')
    .replaceAll("</table>", "</table></div>");
  if (cache.size >= cacheLimit) cache.delete(cache.keys().next().value!);
  cache.set(text, html);
  return html;
}
