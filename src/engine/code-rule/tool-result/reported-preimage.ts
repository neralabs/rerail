/**
 * What a tool's own result says the file it wrote held before it ran. The code-rule bridge reads an edit against this
 * where the call reached Rerail after the tool had written the file, so the call's own capture holds the write.
 */

/**
 * One hunk of the line diff a tool made: the line its lines start at in the file after the tool, counted from 1, the
 * line they started at before it where the result says so, and those lines, each marked ` ` (on both sides), `-`
 * (before only) or `+` (after only), with `\` after a line that ends its file without a line feed. The lines carry no
 * line ending.
 */
export interface ReportedHunk {
  readonly newStart: number;
  readonly oldStart?: number;
  readonly lines: readonly string[];
}

/**
 * What a tool's result says the file it wrote (`path`, as the result names it) held: nothing, as the tool made it
 * (`created`, with the text it wrote where the result gives it), its whole text (`text`), or the line diff the tool made
 * (`diff`), which rebuilds that text from the file as the tool left it, with the whole text too where the result gives
 * both.
 */
export type ReportedPreimage =
  | { readonly kind: "created"; readonly path: string; readonly content?: string }
  | { readonly kind: "text"; readonly path: string; readonly text: string }
  | { readonly kind: "diff"; readonly path: string; readonly hunks: readonly ReportedHunk[]; readonly text?: string };

/** A preimage the result gives text or a line diff for. */
type ReportedEdit = Exclude<ReportedPreimage, { readonly kind: "created" }>;

const BYTE_ORDER_MARK = "\uFEFF";

/** The fields of a Claude Code Write or Edit result that say what the file it names (`filePath`) held. */
interface ClaudeCodeEditResult {
  readonly filePath: string;
  readonly type: unknown;
  readonly content: unknown;
  readonly structuredPatch: unknown;
  readonly originalFile: unknown;
}

/**
 * What a Claude Code Write or Edit result (its `PostToolUse` `tool_response`) says the file held. The result names the
 * file (`filePath`). A Write that made it says `type: "create"`, with the text it wrote (`content`); any other gives the
 * line diff it made (`structuredPatch`) and the text it replaced (`originalFile`), which some results leave null. The
 * line diff comes first, as it is checked against the file (`reportedText`). Undefined when the result says none of these.
 */
export function claudeCodeReportedPreimage(output: unknown): ReportedPreimage | undefined {
  const result = claudeCodeEditResult(output);
  if (result === undefined) return undefined;
  return result.type === "create" ? claudeCodeCreated(result) : claudeCodeReportedEdit(result);
}

/** What the result of a Write that made its file gives: that it made it, and the text it wrote where it says. */
function claudeCodeCreated({ filePath: path, content }: ClaudeCodeEditResult): ReportedPreimage {
  return typeof content === "string" ? { kind: "created", path, content } : { kind: "created", path };
}

/**
 * Whether the call's capture, `captured`, holds other text than the tool wrote, where `reported` says the tool made the
 * file: not its `content` in any form the file may hold it (`inFileForm`, with or without a last line ending). A call
 * taken up only after the tool wrote read that content. One that read other text read the file before the tool ran, the
 * file being removed between the call and the tool, or after a formatter changed the made file; no result tells the two
 * apart, so the edit is read against the capture, as an edit whose result says nothing is. Never where the result does
 * not say what the tool wrote.
 */
export function capturedOtherThanMade(reported: ReportedPreimage, captured: string): boolean {
  if (reported.kind !== "created" || reported.content === undefined) return false;
  const made = inFileForm(reported.content, captured);
  return captured !== made && captured !== `${made}${lineEnding(captured)}`;
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
  const { filePath, originalFile, type, content, structuredPatch } = output as Record<string, unknown>;
  return typeof filePath === "string" && filePath !== "" ? { filePath, type, content, structuredPatch, originalFile } : undefined;
}

/** `structuredPatch` as hunks; undefined when it has none, or holds anything that is no hunk. */
function reportedHunks(patch: unknown): readonly ReportedHunk[] | undefined {
  return Array.isArray(patch) && patch.length > 0 && patch.every(isReportedHunk) ? patch : undefined;
}

function isReportedHunk(hunk: unknown): hunk is ReportedHunk {
  if (typeof hunk !== "object" || hunk === null) return false;
  const { newStart, oldStart, lines } = hunk as Record<string, unknown>;
  return isLineNumber(newStart) && (oldStart === undefined || isLineNumber(oldStart)) && isLineList(lines);
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

/**
 * What the file held before the tool as `reported` says, where the call's own capture, `captured`, already holds what
 * the tool wrote (its call reached Rerail only after the tool ran) and the file changed again since, now `after` (a
 * formatter, the next edit): the text the line diff rebuilds from `captured`. Undefined without a line diff, where
 * `captured` does not hold the diff's lines where the diff places them, as no capture taken before the tool ran does,
 * and where the diff also rebuilds `captured` from `after`, as an edit whose call came on time leaves it, whatever lines
 * it removed.
 */
export function textBeforeWrite(reported: ReportedPreimage, captured: string, after: string | undefined): { readonly text: string } | undefined {
  if (reported.kind !== "diff") return undefined;
  const text = unpatched(captured, reported.hunks);
  return text === undefined || rebuiltText(reported, after) === captured ? undefined : { text };
}

/**
 * What the file held before the tool, for a call whose capture, `captured` (what the call read of the file), the line
 * diff does not place before the tool (`placesBeforeWrite`): nothing for a file the tool made; the text the result
 * gives; or the text the line diff rebuilds from `captured`, which then holds the tool's lines where the diff places
 * them, as a capture taken after the tool wrote does. Undefined when the result says none of these.
 */
export function unprovenCaptureBefore(reported: ReportedPreimage, captured: string, after: string | undefined): { readonly text?: string } | undefined {
  if (reported.kind === "created") return {};
  const given = givenText(reported, after);
  if (given !== undefined) return { text: given };
  const text = reported.kind === "diff" ? unpatched(captured, reported.hunks) : undefined;
  return text === undefined ? undefined : { text };
}

/**
 * Whether `captured`, what a call read of the file, is the file before the tool: it holds each hunk's lines before the
 * diff (` ` and `-`) where the hunk's `oldStart` places them, and, in one hunk at least, a line the tool's write moved or
 * removed, or the file's end, where the tool's lines go (`displacedBy`). A capture taken after the tool wrote holds the
 * tool's lines there, in whatever form a formatter left them. Never without a line diff.
 */
export function placesBeforeWrite(reported: ReportedPreimage, captured: string): boolean {
  if (reported.kind !== "diff") return false;
  const lines = fileLines(captured);
  return reported.hunks.every((hunk) => hunkHeldBefore(lines, hunk) !== undefined) && reported.hunks.some((hunk) => displacedBy(lines, hunk));
}

/**
 * Whether a hunk shows a line of `lines`, the file before the tool, where the tool's write puts its own: a removed line,
 * a line after one the tool added, or, where the tool's lines end the hunk, the file's end right after its lines before
 * the diff.
 */
function displacedBy(lines: readonly string[], hunk: ReportedHunk): boolean {
  const marks = hunk.lines.map((line) => line.charAt(0)).filter((mark) => mark !== "\\");
  const firstAdded = marks.indexOf("+");
  if (marks.includes("-")) return true;
  if (firstAdded < 0) return false;
  if (marks.slice(firstAdded).includes(" ")) return true;
  return Math.max((hunk.oldStart ?? 1) - 1, 0) + firstAdded === lines.length;
}

/** How many lines before the diff a hunk places in `lines` from its `oldStart`; undefined when one is not there. */
function hunkHeldBefore(lines: readonly string[], hunk: ReportedHunk): number | undefined {
  if (hunk.oldStart === undefined) return undefined;
  const from = Math.max(hunk.oldStart - 1, 0);
  let at = from;
  for (const [index, line] of hunk.lines.entries()) {
    if (!line.startsWith(" ") && !line.startsWith("-")) continue;
    if (!heldBeforeLine(line, hunk.lines[index + 1], lines[at])) return undefined;
    at += 1;
  }
  return at - from;
}

/**
 * Whether `held`, the file's line, is the hunk's line before the diff, `line` (` ` or `-`), read as `hunkLine` reads
 * one: `following` marks it as ending its file without a line feed.
 */
function heldBeforeLine(line: string, following: string | undefined, held: string | undefined): boolean {
  const ends = following?.startsWith("\\") !== true;
  return held !== undefined && sameLine(held, line.slice(1).replace(/\r$/u, ""), ends);
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
 * the file's own bytes, and a line only the diff holds takes the file's form (`FileForm`), so a file with CRLF endings
 * is rebuilt with them whether or not the diff's lines carry the carriage return. Undefined when a hunk's lines are not
 * where it places them in `after`, when hunks overlap, or when a line has no mark.
 */
function unpatched(after: string, hunks: readonly ReportedHunk[]): string | undefined {
  const lines = fileLines(after);
  const ending = lineEnding(after);
  const parts: string[] = [];
  let next = 0;
  for (const hunk of [...hunks].sort((left, right) => left.newStart - right.newStart)) {
    const at = Math.max(hunk.newStart - 1, 0);
    const form = { ending, tabs: indentsWithTabs(lines.slice(at, at + heldCount(hunk.lines))) };
    const before = at < next ? undefined : hunkBefore(lines, at, hunk.lines, form);
    if (before === undefined) return undefined;
    parts.push(lines.slice(next, at).join(""), before.text);
    next = before.next;
  }
  parts.push(lines.slice(next).join(""));
  return withMarkOf(parts.join(""), after);
}

/**
 * `text` with the byte order mark `after` starts with. A line diff may show the file's first line without the mark, so a
 * first line put back from it (`-`) lacks it.
 */
function withMarkOf(text: string, after: string): string {
  return after.startsWith(BYTE_ORDER_MARK) && !text.startsWith(BYTE_ORDER_MARK) ? `${BYTE_ORDER_MARK}${text}` : text;
}

/** `text`'s lines, each with its line ending, the last one without where the text ends without one. */
function fileLines(text: string): string[] {
  return Array.from(text.matchAll(/[^\n]*\n|[^\n]+$/gu), (match) => match[0]);
}

/** The line ending `text` uses: CRLF where any line ends with one. */
function lineEnding(text: string): string {
  return text.includes("\r\n") ? "\r\n" : "\n";
}

/**
 * How the file writes a line only the diff holds: with its line `ending`, and with tabs where the diff shows each tab of
 * the run a line starts with as two spaces (`shownIndentation`), when the hunk's lines in the file indent with tabs
 * (`tabs`).
 */
interface FileForm {
  readonly ending: string;
  readonly tabs: boolean;
}

/** How many of the file's lines a hunk's lines cover: those on both sides and those after only. */
function heldCount(hunk: readonly string[]): number {
  return hunk.filter((line) => line.startsWith(" ") || line.startsWith("+")).length;
}

/** Whether any of `lines` starts with a tab. */
function indentsWithTabs(lines: readonly string[]): boolean {
  return lines.some((line) => line.startsWith("\t"));
}

/**
 * `line` as Claude Code's line diff shows it: each tab of the run of tabs it starts with as two spaces. A tab after the
 * first space stays a tab, so a line indented tab, space, tab shows as three spaces and a tab.
 */
function shownIndentation(line: string): string {
  return line.replace(/^\t+/u, (tabs) => "  ".repeat(tabs.length));
}

/** `text` with each pair of spaces of its leading indentation as a tab, as a file that indents with tabs writes it. */
function tabbedIndentation(text: string): string {
  return text.replace(/^(?: {2})+/u, (spaces) => "\t".repeat(spaces.length / 2));
}

/**
 * One hunk's lines before the diff, read against the file's `lines` from `at`, and the index of the file's first line
 * past the hunk. Undefined when the file's lines there are not the hunk's lines after the diff.
 */
function hunkBefore(
  lines: readonly string[],
  at: number,
  hunk: readonly string[],
  form: FileForm,
): { readonly text: string; readonly next: number } | undefined {
  const before: string[] = [];
  let next = at;
  for (const [index, line] of hunk.entries()) {
    const step = hunkLine(line, hunk[index + 1], lines[next], form);
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
 * hunk's next line, which marks this one as ending its file without a line feed. A `-` line puts its text back in the
 * file's `form` (`removedLine`); the others are read by `heldLine`. Undefined when the line does not fit the file there.
 */
function hunkLine(line: string, following: string | undefined, held: string | undefined, form: FileForm): HunkStep | undefined {
  const mark = line.charAt(0);
  if (mark === "\\") return { takes: 0 };
  const text = line.slice(1).replace(/\r$/u, "");
  const ends = following?.startsWith("\\") !== true;
  if (mark === "-") return { before: removedLine(text, ends, form), takes: 0 };
  return heldLine(mark, held, text, ends);
}

/** A line only the diff holds, as the file wrote it before the diff: with the file's tabs and line ending. */
function removedLine(text: string, ends: boolean, form: FileForm): string {
  const line = form.tabs ? tabbedIndentation(text) : text;
  return ends ? `${line}${form.ending}` : line;
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

/**
 * Whether the file's line `held` is the diff's line `text`, as it is or with the tabs it starts with shown as two spaces
 * each (`shownIndentation`), ending in a line feed (with or without a carriage return) when `ends`. A line that starts
 * with a byte order mark, as a file's first line may, is the diff's line with or without it: a line diff may leave the
 * mark out.
 */
function sameLine(held: string, text: string, ends: boolean): boolean {
  const unmarked = held.startsWith(BYTE_ORDER_MARK) ? held.slice(BYTE_ORDER_MARK.length) : held;
  return [held, unmarked].some((line) => sameText(line, text, ends) || sameText(shownIndentation(line), text, ends));
}

function sameText(line: string, text: string, ends: boolean): boolean {
  return ends ? line === `${text}\n` || line === `${text}\r\n` : line === text;
}
