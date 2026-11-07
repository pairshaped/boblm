import { lexer, type Token, type Tokens } from "marked";

const entities: Record<string, string> = {
  "&amp;": "&",
  "&lt;": "<",
  "&gt;": ">",
  "&quot;": '"',
  "&#39;": "'",
};
const decode = (text: string) =>
  text.replace(/&(?:amp|lt|gt|quot|#39);/g, (e) => entities[e]!);

/** Prose only: code, images, raw HTML and link addresses are left out. */
function read(token: Token): string {
  switch (token.type) {
    case "code":
    case "codespan":
      return " ";
    case "html":
    case "image":
    case "def":
    case "space":
    case "hr":
      return "";
    case "br":
      return "\n";
    case "table": {
      const table = token as Tokens.Table;
      const cell = (c: Tokens.TableCell) => c.tokens.map(read).join("");
      return [table.header, ...table.rows]
        .map((row) => row.map(cell).join(" "))
        .join("\n");
    }
    case "list":
      return (token as Tokens.List).items
        .map((item) => item.tokens.map(read).join("\n"))
        .join("\n");
    default: {
      const nested = (token as { tokens?: Token[] }).tokens;
      if (nested) return nested.map(read).join("");
      const text = (token as { text?: unknown }).text;
      return typeof text === "string" ? decode(text) : "";
    }
  }
}

// A shell prompt line is a command, not something to say aloud.
const promptLine =
  /^\s*(?:[$%❯]\s+(?=[A-Za-z_./])|[\w.-]+@[\w.-]+:[^\n]*?[$#]\s|PS\s+[A-Za-z]:\\[^\n]*>\s)/;

const cache = new Map<string, string>();
const cacheLimit = 400;

/** What to read aloud for a message written in markdown; empty when there is nothing worth saying. */
export function speechText(markdown: string): string {
  const hit = cache.get(markdown);
  if (hit !== undefined) return hit;
  const prose = markdown
    .split("\n")
    .filter((line) => !promptLine.test(line))
    .join("\n");
  const text = lexer(prose)
    .map(read)
    .join("\n")
    // Commit hashes read as noise: any long hex string with a digit and a letter, or one named as a commit.
    .replace(
      /\b[0-9a-f]{7,64}\b/gi,
      (value: string, offset: number, source: string) => {
        const full = value.length === 40 || value.length === 64;
        const abbreviated =
          value.length <= 40 && /[a-f]/i.test(value) && /\d/.test(value);
        const named =
          /\b(?:commit|revision|sha(?:-?(?:1|256))?)\s*[:#]?\s*$/i.test(
            source.slice(Math.max(0, offset - 40), offset),
          );
        return full || abbreviated || named ? "" : value;
      },
    )
    .replace(/\(\s*\)|\[\s*\]/g, "")
    .replace(/\s+/g, " ")
    .replace(/\s+([,.;:!?])/g, "$1")
    .trim();
  const readable = /[\p{L}\p{N}]/u.test(text) ? text : "";
  if (cache.size >= cacheLimit) cache.delete(cache.keys().next().value!);
  cache.set(markdown, readable);
  return readable;
}

export function supportsSpeech(): boolean {
  const win = (globalThis as { window?: Window }).window;
  return (
    !!win &&
    typeof win.speechSynthesis?.speak === "function" &&
    typeof win.speechSynthesis.cancel === "function" &&
    typeof (win as { SpeechSynthesisUtterance?: unknown })
      .SpeechSynthesisUtterance === "function"
  );
}

let stopCurrent: (() => void) | undefined;

/** Stop whatever is being read, if anything. */
export function stopSpeaking(): void {
  stopCurrent?.();
}

/**
 * Read text with the browser's default voice. Only one reading plays at a time. `onFinish` runs once, with
 * `true` if the engine failed. Returns a function that stops this reading.
 */
export function speak(
  text: string,
  onFinish: (failed: boolean) => void,
): () => void {
  stopCurrent?.();
  const win = window as unknown as {
    speechSynthesis: SpeechSynthesis;
    SpeechSynthesisUtterance: typeof SpeechSynthesisUtterance;
  };
  const synth = win.speechSynthesis;
  synth.cancel();
  let finished = false;
  let utterance: SpeechSynthesisUtterance | undefined;
  let remaining = text;

  const finish = (failed: boolean) => {
    if (finished) return;
    finished = true;
    if (stopCurrent === stop) stopCurrent = undefined;
    onFinish(failed);
  };
  function stop() {
    if (finished) return;
    finish(false);
    synth.cancel();
  }
  function next() {
    // Short pieces, because some engines refuse or cut off long text.
    let end = Math.min(remaining.length, 240);
    if (end < remaining.length) {
      const boundary = remaining.lastIndexOf(" ", end);
      if (boundary > 0) end = boundary;
      else if (/[\uD800-\uDBFF]/.test(remaining[end - 1]!)) end -= 1;
    }
    const piece = new win.SpeechSynthesisUtterance(remaining.slice(0, end));
    utterance = piece;
    remaining = remaining.slice(end).trimStart();
    piece.onend = () => {
      if (finished || utterance !== piece) return;
      if (!remaining) finish(false);
      else {
        try {
          next();
        } catch {
          finish(true);
        }
      }
    };
    piece.onerror = () => {
      if (!finished && utterance === piece) finish(true);
    };
    synth.speak(piece);
  }

  stopCurrent = stop;
  try {
    next();
  } catch (error) {
    finish(true);
    throw error;
  }
  return stop;
}
