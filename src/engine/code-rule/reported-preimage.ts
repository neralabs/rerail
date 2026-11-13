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
 * as the tool left it, with the whole text too where the result gives both.
 */
export type ReportedPreimage =
  | { readonly kind: "created"; readonly path: string }
  | { readonly kind: "text"; readonly path: string; readonly text: string }
  | { readonly kind: "diff"; readonly path: string; readonly hunks: readonly ReportedHunk[]; readonly text?: string };

/** A preimage the result gives text or a line diff for. */
type ReportedEdit = Exclude<ReportedPreimage, { readonly kind: "created" }>;

const BYTE_ORDER_MARK = "\uFEFF";

/** The fields of a Claude Code Write or Edit result that say what the file it names (`filePath`) held. */
interface ClaudeCodeEditResult {
  readonly filePath: string;
  readonly type: unknown;
  readonly structuredPatch: unknown;
  readonly originalFile: unknown;
}

/**
 * What a Claude Code Write or Edit result (its `PostToolUse` `tool_response`) says the file held. The result names the
 * file (`filePath`). A Write that made it says `type: "create"`; any other gives the line diff it made
 * (`structuredPatch`) and the text it replaced (`originalFile`), which some results leave null. The line diff comes
 * first, as it is checked against the file (`reportedText`). Undefined when the result says none of these.
 */
export function claudeCodeReportedPreimage(output: unknown): ReportedPreimage | undefined {
  const result = claudeCodeEditResult(output);
  if (result === undefined) return undefined;
  return result.type === "create" ? { kind: "created", path: result.filePath } : claudeCodeReportedEdit(result);
}

/** What the result of an Edit, or of a Write that replaced the file, gives: the line diff, the text it replaced, or both. */
function claudeCodeReportedEdit({ filePath: path, structuredPatch, originalFile }: ClaudeCodeEditResult): ReportedEdit | undefined {
  const hunks = reportedHunks(structuredPatch);
  const text = typeof originalFile === "string" ? originalFile : undefined;
  if (hunks === undefined) return text === undefined ? undefined : { kind: "text", path, text };
  return text === undefined ? { kind: "diff", path, hunks } : { kind: "diff", path, hunks, text };
}

/** `output`'s fields as a Write or Edit result's; undefined unless it names a file. */
function claudeCodeEditResult(output: unknown): ClaudeCodeEditResult | undefined {
  if (typeof output !== "object" || output === null) return undefined;
  const { filePath, originalFile, type, structuredPatch } = output as Record<string, unknown>;
  return typeof filePath === "string" && filePath !== "" ? { filePath, type, structuredPatch, originalFile } : undefined;
}

/** `structuredPatch` as hunks; undefined when it has none, or holds anything that is no hunk. */
function reportedHunks(patch: unknown): readonly ReportedHunk[] | undefined {
  return Array.isArray(patch) && patch.length > 0 && patch.every(isReportedHunk) ? patch : undefined;
}

function isReportedHunk(hunk: unknown): hunk is ReportedHunk {
  if (typeof hunk !== "object" || hunk === null) return false;
  const { newStart, lines } = hunk as Record<string, unknown>;
  return isLineNumber(newStart) && isLineList(lines);
}

/** A hunk's first line: a whole number from 0, which a diff that empties the file gives. */
function isLineNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function isLineList(value: unknown): value is readonly string[] {
  return Array.isArray(value) && value.every((line) => typeof line === "string");
}

/**
 * What the file held before the tool as `reported` says, given `after`, its text as the tool left it: no text for a
 * file the tool made, the text the line diff rebuilds from `after`, or else the text given. Undefined when there is no
 * text given and `after` does not hold the diff's lines where the diff places them, as when the file changed again
 * after the tool wrote it.
 */
export function reportedText(reported: ReportedPreimage, after: string | undefined): { readonly text?: string } | undefined {
  if (reported.kind === "created") return {};
  const text = rebuiltText(reported, after) ?? givenText(reported, after);
  return text === undefined ? undefined : { text };
}

/** The text a line diff rebuilds from `after`; undefined without a line diff, or where `after` does not fit it. */
function rebuiltText(reported: ReportedEdit, after: string | undefined): string | undefined {
  return reported.kind === "diff" && after !== undefined ? unpatched(after, reported.hunks) : undefined;
}

/** The text the result gives, in the form of `after` where there is one (`inFileForm`). */
function givenText(reported: ReportedEdit, after: string | undefined): string | undefined {
  return reported.text === undefined || after === undefined ? reported.text : inFileForm(reported.text, after);
}

/**
 * `text` with the byte order mark of `after`, and with its CRLF line endings where `text` has none of its own. A result
 * may give the text a tool replaced without the mark, or with LF where the file has CRLF; read as it is, an edit
 * against it would count those lines as changed, and a Write that left the file as it was would be an edit.
 */
function inFileForm(text: string, after: string): string {
  const unmarked = text.startsWith(BYTE_ORDER_MARK) ? text.slice(1) : text;
  const ended = lineEnding(after) === "\r\n" && !unmarked.includes("\r\n") ? unmarked.replace(/\n/gu, "\r\n") : unmarked;
  return after.startsWith(BYTE_ORDER_MARK) ? `${BYTE_ORDER_MARK}${ended}` : ended;
}

/**
 * The text a line diff made `after` from: each hunk's lines put back as they were before it. Lines on both sides keep
 * the file's own bytes, and a line only the diff holds takes the file's line ending, so a file with CRLF endings is
 * rebuilt with them whether or not the diff's lines carry the carriage return. Undefined when a hunk's lines are not
 * where it places them in `after`, when hunks overlap, or when a line has no mark.
 */
function unpatched(after: string, hunks: readonly ReportedHunk[]): string | undefined {
  const lines = Array.from(after.matchAll(/[^\n]*\n|[^\n]+$/gu), (match) => match[0]);
  const ending = lineEnding(after);
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

/** The line ending `text` uses: CRLF where any line ends with one. */
function lineEnding(text: string): string {
  return text.includes("\r\n") ? "\r\n" : "\n";
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
    const step = hunkLine(line, hunk[index + 1], lines[next], ending);
    if (step === undefined) return undefined;
    if (step.before !== undefined) before.push(step.before);
    next += step.takes;
  }
  return { text: before.join(""), next };
}

/** What one line of a hunk puts back before the diff (`before`), and how many of the file's lines it takes up. */
interface HunkStep {
  readonly before?: string;
  readonly takes: 0 | 1;
}

/**
 * One line of a hunk, read where the hunk has got to in the file: `held` is the file's line there, and `following` the
 * hunk's next line, which marks this one as ending its file without a line feed. A `-` line puts its text back with
 * the file's line `ending`; the others are read by `heldLine`. Undefined when the line does not fit the file there.
 */
function hunkLine(line: string, following: string | undefined, held: string | undefined, ending: string): HunkStep | undefined {
  const mark = line.charAt(0);
  if (mark === "\\") return { takes: 0 };
  const text = line.slice(1).replace(/\r$/u, "");
  const ends = following?.startsWith("\\") !== true;
  if (mark === "-") return { before: ends ? `${text}${ending}` : text, takes: 0 };
  return heldLine(mark, held, text, ends);
}

/**
 * A hunk line the file after the diff holds (` ` or `+`), which must be the file's line `held`: a ` ` line puts that
 * line back before the diff too. Undefined when `held` is not the line, or the line has no mark.
 */
function heldLine(mark: string, held: string | undefined, text: string, ends: boolean): HunkStep | undefined {
  if (held === undefined || !sameLine(held, text, ends)) return undefined;
  if (mark === " ") return { before: held, takes: 1 };
  return mark === "+" ? { takes: 1 } : undefined;
}

/** Whether the file's line `held` is the diff's line `text`, ending in a line feed (with or without a carriage return) when `ends`. */
function sameLine(held: string, text: string, ends: boolean): boolean {
  return ends ? held === `${text}\n` || held === `${text}\r\n` : held === text;
}
