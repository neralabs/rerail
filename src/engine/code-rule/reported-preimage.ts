/**
 * What a tool's own result says the file it wrote held before it ran. The code-rule bridge reads an edit against this
 * only where the call reached Rerail after the tool had written the file, so the call's own capture holds the write.
 */

/**
 * One hunk of the line diff a tool made: the line its lines start at in the file after the tool, counted from 1, and
 * those lines, each marked ` ` (on both sides), `-` (before only) or `+` (after only), with `\` after a line that ends
 * its file without a line feed. The lines carry no line ending.
 */
export interface ReportedHunk {
  readonly newStart: number;
  readonly lines: readonly string[];
}

/**
 * What a tool's result says the file it wrote (`path`, as the result names it) held: nothing, as the tool made it
 * (`created`), its whole text (`text`), or the line diff the tool made (`diff`), which rebuilds that text from the file
 * as the tool left it.
 */
export type ReportedPreimage =
  | { readonly kind: "created"; readonly path: string }
  | { readonly kind: "text"; readonly path: string; readonly text: string }
  | { readonly kind: "diff"; readonly path: string; readonly hunks: readonly ReportedHunk[] };

/**
 * What a Claude Code Write or Edit result (its `PostToolUse` `tool_response`) says the file held. The result names the
 * file (`filePath`). A Write that made it says `type: "create"`; any other gives the line diff it made
 * (`structuredPatch`) and the text it replaced (`originalFile`), which some results leave null. The line diff comes
 * first, as it is checked against the file (`reportedText`). Undefined when the result says none of these.
 */
export function claudeCodeReportedPreimage(output: unknown): ReportedPreimage | undefined {
  if (typeof output !== "object" || output === null) return undefined;
  const { filePath, originalFile, type, structuredPatch } = output as Record<string, unknown>;
  if (typeof filePath !== "string" || filePath === "") return undefined;
  if (type === "create") return { kind: "created", path: filePath };
  const hunks = reportedHunks(structuredPatch);
  if (hunks !== undefined) return { kind: "diff", path: filePath, hunks };
  return typeof originalFile === "string" ? { kind: "text", path: filePath, text: originalFile } : undefined;
}

/** `structuredPatch` as hunks; undefined when it has none, or holds anything that is no hunk. */
function reportedHunks(patch: unknown): readonly ReportedHunk[] | undefined {
  return Array.isArray(patch) && patch.length > 0 && patch.every(isReportedHunk) ? patch : undefined;
}

function isReportedHunk(hunk: unknown): hunk is ReportedHunk {
  if (typeof hunk !== "object" || hunk === null) return false;
  const { newStart, lines } = hunk as Record<string, unknown>;
  return typeof newStart === "number" && Number.isSafeInteger(newStart) && newStart >= 0
    && Array.isArray(lines) && lines.every(line => typeof line === "string");
}

/**
 * What the file held before the tool as `reported` says, given `after`, its text as the tool left it: no text for a
 * file the tool made, the text given, or the text the line diff rebuilds from `after`. Undefined when `after` does not
 * hold the diff's lines where the diff places them, as when the file changed again after the tool wrote it.
 */
export function reportedText(reported: ReportedPreimage, after: string | undefined): { readonly text?: string } | undefined {
  if (reported.kind === "created") return {};
  if (reported.kind === "text") return { text: reported.text };
  const text = after === undefined ? undefined : unpatched(after, reported.hunks);
  return text === undefined ? undefined : { text };
}

/**
 * The text a line diff made `after` from: each hunk's lines put back as they were before it. Lines on both sides keep
 * the file's own bytes, and a line only the diff holds takes the file's line ending, so a file with CRLF endings is
 * rebuilt with them whether or not the diff's lines carry the carriage return. Undefined when a hunk's lines are not
 * where it places them in `after`, when hunks overlap, or when a line has no mark.
 */
function unpatched(after: string, hunks: readonly ReportedHunk[]): string | undefined {
  const lines = after.match(/[^\n]*\n|[^\n]+$/gu) ?? [];
  const ending = after.includes("\r\n") ? "\r\n" : "\n";
  const parts: string[] = [];
  let next = 0;
  for (const hunk of [...hunks].sort((left, right) => left.newStart - right.newStart)) {
    const at = Math.max(hunk.newStart - 1, 0);
    const before = at < next ? undefined : hunkBefore(lines, at, hunk.lines, ending);
    if (before === undefined) return undefined;
    parts.push(lines.slice(next, at).join(""), before.text);
    next = before.next;
  }
  parts.push(lines.slice(next).join(""));
  return parts.join("");
}

/**
 * One hunk's lines before the diff, read against the file's `lines` from `at`, and the index of the file's first line
 * past the hunk. Undefined when the file's lines there are not the hunk's lines after the diff.
 */
function hunkBefore(
  lines: readonly string[],
  at: number,
  hunk: readonly string[],
  ending: string,
): { readonly text: string; readonly next: number } | undefined {
  const before: string[] = [];
  let next = at;
  for (const [index, line] of hunk.entries()) {
    const mark = line.charAt(0);
    if (mark === "\\") continue;
    if (mark !== " " && mark !== "-" && mark !== "+") return undefined;
    const text = line.slice(1).replace(/\r$/u, "");
    const ends = hunk[index + 1]?.startsWith("\\") !== true;
    if (mark === "-") {
      before.push(ends ? `${text}${ending}` : text);
      continue;
    }
    const held = lines[next];
    if (held === undefined || !sameLine(held, text, ends)) return undefined;
    next += 1;
    if (mark === " ") before.push(held);
  }
  return { text: before.join(""), next };
}

/** Whether the file's line `held` is the diff's line `text`, ending in a line feed (with or without a carriage return) when `ends`. */
function sameLine(held: string, text: string, ends: boolean): boolean {
  return ends ? held === `${text}\n` || held === `${text}\r\n` : held === text;
}
