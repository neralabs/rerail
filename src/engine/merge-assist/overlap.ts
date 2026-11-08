/**
 * Overlapping work: a read-only identifier of the textual conflicts one side's changes would have with another ref's.
 *
 * No merge is run and nothing is written to any repository. Both sides are `git diff --unified=0` hunks as line ranges
 * of the base: this side's working files against the base, and another ref's own work (its diff from where it meets
 * its base branch, as a pull request shows it, so mainline work it merged in does not count) carried to the base's
 * lines through that point's own diff to the base. Two hunks in the same file conflict when they touch the same base
 * lines or leave at most {@link ADJACENT_LINES} unchanged lines between them, which approximates git's own conflict
 * rule; the same change on both sides does not conflict. Renames are ignored.
 */
import { createHash } from "node:crypto";
import { runGit } from "../git.js";

/** The most unchanged base lines two hunks may leave between them and still conflict. */
export const ADJACENT_LINES = 3;
/** The most lines of one side's change one conflict shows. */
export const HUNK_LINES = 40;
/** A binary file's change covers the whole file. */
const WHOLE_FILE = 1e9;
const DIFF_OPTIONS = { maxBuffer: 512 * 1024 * 1024, timeout: 60_000 };

/**
 * A hunk's lines in the base, as a closed range: `lo..hi` for changed lines, `s + 0.5` for an insertion after line
 * `s` (0.5 for a file the base does not have), and a digest of its `-`/`+` lines (`sig`): two sides making the same
 * change to the same lines do not conflict, as in git.
 */
export interface LineRange {
  readonly lo: number;
  readonly hi: number;
  readonly sig: string;
}

/** One hunk of a change: its base range and its text (the `@@` line and its `-`/`+` lines, clipped). */
export interface RefHunk extends LineRange {
  readonly text: string;
}

/** One identified conflict: a ref, a file both sides change, that ref's overlapping hunks and this side's. */
export interface GitConflict {
  readonly ref: string;
  readonly file: string;
  readonly theirs: readonly RefHunk[];
  readonly ours: readonly LineRange[];
}

/** A read-only git command's output; null when it fails. */
function gitRead(cwd: string, args: readonly string[]): string | null {
  const result = runGit(cwd, ["--no-optional-locks", "-c", "core.quotePath=false", ...args], DIFF_OPTIONS);
  return result.status === 0 ? result.stdout : null;
}

/** One parsed `@@` hunk: both sides' starts and counts, and its text. */
interface RawHunk {
  readonly oldStart: number;
  readonly oldCount: number;
  readonly newStart: number;
  readonly newCount: number;
  readonly lines: string[];
}

/** A parsed hunk as a base range with its digest. */
function rangeOf(hunk: RawHunk): LineRange {
  const sig = createHash("sha1").update(hunk.lines.slice(1).join("\n")).digest("hex").slice(0, 16);
  const start = hunk.oldStart;
  return hunk.oldCount === 0 ? { lo: start + 0.5, hi: start + 0.5, sig } : { lo: start, hi: start + hunk.oldCount - 1, sig };
}

const HEADER = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/u;
const BINARY = /^Binary files (?:a\/(.+)|\/dev\/null) and (?:b\/(.+)|\/dev\/null) differ$/u;

function unquote(path: string): string {
  return path.startsWith("\"") && path.endsWith("\"") ? path.slice(1, -1) : path;
}

/** A diff being parsed: its files so far, the file and hunk lines go to, and the current file's header fields. */
interface DiffParse {
  readonly files: Map<string, RawHunk[]>;
  oldPath: string;
  blobs: string;
  current: RawHunk[] | null;
  hunk: RawHunk | null;
}

/** A header line before a file's first hunk: `index`, `---` or `+++`. */
function headerLine(state: DiffParse, line: string): boolean {
  if (state.hunk !== null) return false;
  if (line.startsWith("index ")) state.blobs = line;
  else if (line.startsWith("--- ")) state.oldPath = line === "--- /dev/null" ? "" : unquote(line.slice(4)).replace(/^a\//u, "");
  else if (line.startsWith("+++ ")) {
    const path = line === "+++ /dev/null" ? state.oldPath : unquote(line.slice(4)).replace(/^b\//u, "");
    state.current = state.files.get(path) ?? [];
    state.files.set(path, state.current);
  } else return false;
  return true;
}

/** A binary file's change covers the whole file; the blob ids tell two binary changes apart. */
function binaryLine(state: DiffParse, line: string): void {
  const match = BINARY.exec(line);
  const path = match?.[2] ?? match?.[1];
  if (path !== undefined) state.files.set(path, [{ oldStart: 0, oldCount: WHOLE_FILE, newStart: 0, newCount: WHOLE_FILE, lines: ["(binary file changed)", state.blobs] }]);
}

function hunkHeader(state: DiffParse, line: string): void {
  const match = HEADER.exec(line);
  if (match === null || state.current === null) return;
  state.hunk = { oldStart: Number(match[1]), oldCount: match[2] === undefined ? 1 : Number(match[2]), newStart: Number(match[3]), newCount: match[4] === undefined ? 1 : Number(match[4]), lines: [line] };
  state.current.push(state.hunk);
}

function parseLine(state: DiffParse, line: string): void {
  if (line.startsWith("diff --git ")) {
    Object.assign(state, { current: null, hunk: null, oldPath: "", blobs: "" });
  } else if (headerLine(state, line)) {
    return;
  } else if (line.startsWith("Binary files ")) {
    binaryLine(state, line);
  } else if (line.startsWith("@@ ")) {
    hunkHeader(state, line);
  } else if (state.hunk !== null && (line.startsWith("+") || line.startsWith("-") || line.startsWith("\\"))) {
    state.hunk.lines.push(line);
  }
}

/** `git diff --unified=0 --no-renames` output as hunks per file (the post-image path, or the pre-image for a deletion). */
function parseDiff(text: string): Map<string, RawHunk[]> {
  const state: DiffParse = { files: new Map(), oldPath: "", blobs: "", current: null, hunk: null };
  for (const line of text.split("\n")) parseLine(state, line);
  return state.files;
}

/** `git diff --unified=0` between two commits (or a commit and the working files, `to` null), limited to `paths` when given. */
function diffHunks(repo: string, from: string, to: string | null, paths?: readonly string[]): Map<string, RawHunk[]> {
  if (paths !== undefined && paths.length === 0) return new Map();
  const args = ["diff", "--no-color", "--no-ext-diff", "--no-renames", "--unified=0", "--src-prefix=a/", "--dst-prefix=b/", from, ...(to === null ? [] : [to])];
  const text = gitRead(repo, paths === undefined ? args : [...args, "--", ...paths.map(path => `:(literal)${path}`)]);
  if (text === null) throw new Error(`git diff ${from} ${to ?? "(working files)"} failed in ${repo}`);
  return parseDiff(text);
}

/**
 * A line of the merge base carried to the base through the merge base's hunks against it: shifted past the hunks
 * before it, or the base's side of the hunk that changed it.
 */
function carry(point: number, hunks: readonly RawHunk[]): { lo: number; hi: number } {
  let offset = 0;
  for (const hunk of hunks) {
    const last = hunk.oldCount === 0 ? hunk.oldStart : hunk.oldStart + hunk.oldCount - 1;
    if (hunk.oldCount > 0 && point >= hunk.oldStart && point <= last) {
      return hunk.newCount === 0 ? { lo: hunk.newStart + 0.5, hi: hunk.newStart + 0.5 } : { lo: hunk.newStart, hi: hunk.newStart + hunk.newCount - 1 };
    }
    if (point > last) offset += hunk.newCount - hunk.oldCount;
    else break;
  }
  return { lo: point + offset, hi: point + offset };
}

/** At most `limit` of `lines`, with a count of the rest. */
export function clipLines(lines: readonly string[], limit: number): string {
  return lines.length <= limit ? lines.join("\n") : [...lines.slice(0, limit), `[${lines.length - limit} more lines]`].join("\n");
}

/** A raw hunk as a base range with its clipped text. */
function refHunk(hunk: RawHunk): RefHunk {
  return { ...rangeOf(hunk), text: clipLines(hunk.lines, HUNK_LINES) };
}

/**
 * `sha`'s own changes, from `own` (where its work starts: {@link ownBase}), as `base` lines: each hunk's range in `own`
 * carried to `base` through `own`'s diff to `base`, file by file. Work `sha` merged in from its base branch after `own`
 * is not its own and is not counted. `paths` limits the files read.
 */
export function refHunks(repo: string, base: string, own: string, sha: string, paths?: readonly string[]): Record<string, RefHunk[]> {
  const theirs = diffHunks(repo, own, sha, paths);
  const drift = own === base || theirs.size === 0 ? new Map<string, RawHunk[]>() : diffHunks(repo, own, base, [...theirs.keys()]);
  const files: Record<string, RefHunk[]> = {};
  for (const [path, hunks] of theirs) {
    const moves = drift.get(path) ?? [];
    files[path] = hunks.map(hunk => {
      const range = refHunk(hunk);
      if (moves.length === 0) return range;
      return { ...range, lo: carry(range.lo, moves).lo, hi: carry(range.hi, moves).hi };
    });
  }
  return files;
}

/** The files `sha` changes from `own`, where its own work starts. */
export function refFiles(repo: string, own: string, sha: string): string[] {
  return (gitRead(repo, ["diff", "--name-only", "--no-renames", own, sha]) ?? "").split("\n").filter(path => path !== "");
}

/**
 * Where `sha`'s own work starts now: its merge base with the base branch's tip (`baseTip`), so mainline work it merged
 * in is not its own; or, when that is nearer, its merge base with `checkoutBase` (where this side's work starts), so
 * work already in this side's base is not its own either. Null when the base branch already holds all of `sha` (merged,
 * or the base branch itself) or the two share no history.
 */
export function ownBase(repo: string, sha: string, baseTip: string, checkoutBase: string): string | null {
  const mergeBase = (a: string, b: string): string | null => gitRead(repo, ["merge-base", "--end-of-options", a, b])?.trim().split("\n")[0] || null;
  const own = mergeBase(sha, baseTip);
  if (own === null) return mergeBase(sha, checkoutBase);
  if (own === sha) return null;
  const shared = mergeBase(checkoutBase, sha);
  const nearer = shared !== null && runGit(repo, ["merge-base", "--is-ancestor", "--end-of-options", own, shared]).status === 0;
  return nearer ? shared : own;
}

/** The working files' changes against `base`, untracked files as new files, as base ranges with their text. Reads only. */
export function workspaceRanges(root: string, base: string): Map<string, RefHunk[]> {
  const ranges = new Map<string, RefHunk[]>();
  for (const [path, hunks] of diffHunks(root, base, null)) ranges.set(path, hunks.map(refHunk));
  const untracked = gitRead(root, ["ls-files", "--others", "--exclude-standard", "-z"]) ?? "";
  for (const path of untracked.split("\0").filter(item => item !== "")) ranges.set(path, [{ lo: 0.5, hi: 0.5, sig: "untracked", text: "(new file)" }]);
  return ranges;
}

/** The changes between two commits, limited to `paths`, as base ranges. */
export function commitRanges(repo: string, base: string, head: string, paths: readonly string[]): Map<string, LineRange[]> {
  const ranges = new Map<string, LineRange[]>();
  for (const [path, hunks] of diffHunks(repo, base, head, paths)) ranges.set(path, hunks.map(rangeOf));
  return ranges;
}

/**
 * Whether two changes conflict: they touch the same base lines or leave at most {@link ADJACENT_LINES} unchanged lines
 * between them, unless they are the same change to the same lines.
 */
export function overlaps(a: LineRange, b: LineRange): boolean {
  if (a.lo === b.lo && a.hi === b.hi && a.sig === b.sig) return false;
  return Math.max(a.lo, b.lo) - Math.min(a.hi, b.hi) - 1 <= ADJACENT_LINES;
}

/** Every (ref, file) where one of this side's hunks overlaps one of the ref's, with the overlapping hunks of both. */
export function identify<Range extends LineRange>(
  agent: ReadonlyMap<string, readonly Range[]>,
  refs: readonly { readonly ref: string; readonly files: Readonly<Record<string, readonly RefHunk[]>> }[],
): Array<GitConflict & { readonly ours: readonly Range[] }> {
  const conflicts: Array<GitConflict & { readonly ours: readonly Range[] }> = [];
  for (const side of refs) {
    for (const [file, mine] of agent) {
      const theirs = (side.files[file] ?? []).filter(hunk => mine.some(range => overlaps(range, hunk)));
      if (theirs.length === 0) continue;
      conflicts.push({ ref: side.ref, file, theirs, ours: mine.filter(range => theirs.some(hunk => overlaps(range, hunk))) });
    }
  }
  return conflicts;
}

/** A conflict's key in hook state and logs. */
export function conflictKey(conflict: Pick<GitConflict, "ref" | "file">): string {
  return `${conflict.ref}\u0000${conflict.file}`;
}
