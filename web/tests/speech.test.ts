import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><body></body>");
type Utterance = {
  text: string;
  onend?: () => void;
  onerror?: () => void;
};
const spoken: Utterance[] = [];
let cancelled = 0;
Object.assign(dom.window, {
  speechSynthesis: {
    speak: (u: Utterance) => spoken.push(u),
    cancel: () => {
      cancelled++;
    },
  },
  SpeechSynthesisUtterance: class {
    text: string;
    constructor(text: string) {
      this.text = text;
    }
  },
});
Object.assign(globalThis, { window: dom.window });
const { speechText, speak, supportsSpeech } = await import("../src/speech.ts");

test("prose is read, code and the rest of the markup are not", () => {
  const text = speechText(
    [
      "# Heading",
      "",
      "Run `npm test` and then:",
      "",
      "```sh",
      "rm -rf /",
      "```",
      "",
      "- first item",
      "- second item",
      "",
      "See [the docs](https://example.com) and ![a chart](chart.png).",
      "",
      "<div>raw html</div>",
    ].join("\n"),
  );
  assert.match(text, /Heading/);
  assert.match(text, /first item/);
  assert.match(text, /second item/);
  assert.match(text, /the docs/);
  assert.doesNotMatch(text, /npm test|rm -rf|https:|chart|raw html|`/);
});

test("table cells are read as text", () => {
  const text = speechText("| name | value |\n|---|---|\n| alpha | 1 |\n");
  assert.match(text, /alpha/);
  assert.doesNotMatch(text, /\||---/);
});

test("terminal prompt lines and commit hashes are skipped, ordinary numbers stay", () => {
  const text = speechText(
    "Done in 3 steps.\n\n$ git status\nuser@host:~/work$ ls\n\nCommit 1a2b3c4d5e fixed version 1.2.3 and 2026.",
  );
  assert.match(text, /Done in 3 steps/);
  assert.match(text, /version 1\.2\.3 and 2026/);
  assert.doesNotMatch(text, /git status|user@host|1a2b3c4d5e/);
});

test("text with nothing to say is empty, so no button is offered", () => {
  assert.equal(speechText("```sh\nls\n```"), "");
  assert.equal(speechText("`only code`"), "");
  assert.equal(speechText("![img](x.png)"), "");
  assert.equal(speechText(""), "");
});

test("support is detected from the browser's speech API", () => {
  assert.equal(supportsSpeech(), true);
});

test("long text is read in short pieces cut at spaces, one after another", () => {
  spoken.length = 0;
  const words = Array.from({ length: 120 }, (_, i) => `word${i}`).join(" ");
  let finished: boolean | undefined;
  speak(words, (failed) => {
    finished = failed;
  });
  assert.equal(spoken.length, 1, "the next piece waits for the first to end");
  const pieces: string[] = [];
  while (spoken.length) {
    const piece = spoken.shift()!;
    pieces.push(piece.text);
    assert.ok(piece.text.length <= 240, `${piece.text.length} characters`);
    piece.onend?.();
  }
  assert.equal(pieces.join(" "), words, "nothing is lost or repeated");
  assert.ok(pieces.length > 1);
  assert.equal(finished, false);
});

test("starting a new reading stops the one before it", () => {
  spoken.length = 0;
  const events: string[] = [];
  speak("first message", (failed) => events.push(`first:${failed}`));
  speak("second message", (failed) => events.push(`second:${failed}`));
  assert.deepEqual(events, ["first:false"], "the first ends without error");
  assert.equal(spoken.at(-1)?.text, "second message");
  spoken.at(-1)?.onend?.();
  assert.deepEqual(events, ["first:false", "second:false"]);
});

test("stop ends the reading once and cancels the speech engine", () => {
  spoken.length = 0;
  cancelled = 0;
  const events: boolean[] = [];
  const stop = speak("a message", (failed) => events.push(failed));
  const before = cancelled;
  stop();
  stop();
  assert.deepEqual(events, [false]);
  assert.ok(cancelled > before);
});

test("an engine error reports failure", () => {
  spoken.length = 0;
  let failed: boolean | undefined;
  speak("a message", (f) => {
    failed = f;
  });
  spoken.at(-1)?.onerror?.();
  assert.equal(failed, true);
});
