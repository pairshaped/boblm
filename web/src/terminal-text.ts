// Tool output from Codex can carry the control sequences its terminal produced.
// Drop them so the timeline shows the text instead of escape junk.

/** An OSC sequence ends at BEL, at ESC \, or at the end of the string. */
const osc = /\x1b\][\s\S]*?(?:\x07|\x1b\\|$)/g;
/** CSI: parameters and intermediates, then one final byte. */
const csi = /\x1b\[[0-?]*[ -/]*[@-~]/g;
/** Any other two-character escape, such as ESC 7 and ESC 8. */
const escape = /\x1b[\s\S]/g;
/** Stray C0 controls and DEL, keeping newline and tab. */
const controls = /[\x00-\x08\x0B\x0C\x0E-\x1F\x7F]/g;

export function cleanTerminalText(text: string): string {
  return text
    .replace(osc, "")
    .replace(csi, "")
    .replace(escape, "")
    .replace(/\r\n/g, "\n")
    .replace(/\r/g, "")
    .replace(controls, "");
}
