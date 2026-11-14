/**
 * Overlapping work: a read-only identifier of the textual conflicts one side's changes would have with another ref's.
 *
 * No merge is run and nothing is written to any repository. Both sides are `git diff --unified=0` hunks as line ranges
 * of the base: this side's working files against the base, and another ref's own work (its diff from where it meets
 * its base branch, as a pull request shows it, so mainline work it merged in does not count) carried to the base's
 * lines through that point's own diff to the base; a hunk of it the base branch's tip already holds (a squash merge
 * that left the branch behind) is not live work and is left out. Two hunks in the same file conflict when they touch
 * the same base lines or leave at most {@link ADJACENT_LINES} unchanged lines between them, which approximates git's
 * own conflict rule; the same change on both sides does not conflict. Renames are ignored. Every read runs without
 * blocking, and no diff runs a program a repository's config names (external diff, textconv).
 */
import { createHash } from "node:crypto";
import { gitReadAsync, mergeBaseAsync } from "./branch-git.js";

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

/** A diff's options: no colour, no external diff or textconv program (a repository's config would choose them), no renames. */
const DIFF_ARGS = ["diff", "--no-color", "--no-ext-diff", "--no-textconv", "--no-renames"] as const;
/** Paths one `git diff` names on its command line: far below any system's argument limit. */
const PATHS_PER_DIFF = 200;

/** `git diff --unified=0` between a commit and the working files, or between two commits limited to `paths`. */
async function diffText(repo: string, from: string, to: string | null, paths: readonly string[] = []): Promise<Map<string, RawHunk[]>> {
  const args = [...DIFF_ARGS, "--unified=0", "--src-prefix=a/", "--dst-prefix=b/", "--end-of-options", from, ...(to === null ? [] : [to])];
  const text = await gitReadAsync(repo, paths.length === 0 ? args : [...args, "--", ...paths.map(path => `:(literal)${path}`)], DIFF_OPTIONS);
  if (text === undefined) throw new Error(`git diff ${from} ${to ?? "(working files)"} failed in ${repo}`);
  return parseDiff(text);
}

/**
 * The hunks of `from..to` in those of `paths` it changes. Git lists the changed files first and reads only the ones in
 * `paths`, a few at a time, so neither a long list of paths nor a large diff elsewhere reaches a command line or memory.
 */
async function diffHunks(repo: string, from: string, to: string, paths: ReadonlySet<string>): Promise<Map<string, RawHunk[]>> {
  const listed = await gitReadAsync(repo, [...DIFF_ARGS, "--name-only", "-z", "--end-of-options", from, to], DIFF_OPTIONS);
  if (listed === undefined) throw new Error(`git diff ${from} ${to} failed in ${repo}`);
  const changed = listed.split("\0").filter(path => path !== "" && paths.has(path));
  const hunks = new Map<string, RawHunk[]>();
  for (let index = 0; index < changed.length; index += PATHS_PER_DIFF) {
    for (const [path, fileHunks] of await diffText(repo, from, to, changed.slice(index, index + PATHS_PER_DIFF))) hunks.set(path, fileHunks);
  }
  return hunks;
}

/** A hunk's last line in the first commit (an insertion's: the line it follows). */
const lastOld = (hunk: RawHunk): number => (hunk.oldCount === 0 ? hunk.oldStart : hunk.oldStart + hunk.oldCount - 1);

/** Whether a hunk changed or removed `point`, a line of the first commit. */
const changes = (hunk: RawHunk, point: number): boolean => hunk.oldCount > 0 && point >= hunk.oldStart && point <= lastOld(hunk);

/** A hunk's lines in the second commit: `lo > hi` when it removed its lines. */
function newLines(hunk: RawHunk): { lo: number; hi: number; moved: boolean } {
  return { lo: hunk.newCount === 0 ? hunk.newStart + 1 : hunk.newStart, hi: hunk.newStart + hunk.newCount - 1, moved: true };
}

/**
 * Where a line of one commit stands in another, through the first's hunks against the second: shifted past the hunks
 * before it, or, for a line a hunk changed, that hunk's lines in the second (`lo > hi` when it removed them) and
 * `moved`.
 */
function carry(point: number, hunks: readonly RawHunk[]): { lo: number; hi: number; moved: boolean } {
  const changing = hunks.find(hunk => changes(hunk, point));
  if (changing !== undefined) return newLines(changing);
  const offset = hunks.filter(hunk => point > lastOld(hunk)).reduce((sum, hunk) => sum + hunk.newCount - hunk.oldCount, 0);
  return { lo: point + offset, hi: point + offset, moved: false };
}

/** A range carried through hunks: from where its first line goes to where its last does; a removed line, to an insertion. */
function carryRange(range: { readonly lo: number; readonly hi: number }, hunks: readonly RawHunk[]): { lo: number; hi: number } {
  const at = (point: number): { lo: number; hi: number } => {
    const carried = carry(point, hunks);
    return carried.lo > carried.hi ? { lo: carried.lo - 0.5, hi: carried.lo - 0.5 } : carried;
  };
  return { lo: at(range.lo).lo, hi: at(range.hi).hi };
}

/** Whether `needle` is a run of consecutive lines of `haystack`. */
function containsRun(haystack: readonly string[], needle: readonly string[]): boolean {
  for (let start = 0; start + needle.length <= haystack.length; start += 1) {
    if (needle.every((line, index) => haystack[start + index] === line)) return true;
  }
  return false;
}

/**
 * Where a hunk's lines stand in another commit, through the hunks `moves` from the first to it: from just past the
 * line before the hunk to just short of the line after (1-based, inclusive), widened to a neighbour's own hunk where
 * `moves` changed that neighbour too.
 */
function hunkWindow(hunk: RawHunk, moves: readonly RawHunk[]): { lo: number; hi: number } {
  const insertion = hunk.oldCount === 0;
  const before = carry(insertion ? hunk.oldStart : hunk.oldStart - 1, moves);
  const after = carry(insertion ? hunk.oldStart + 1 : hunk.oldStart + hunk.oldCount, moves);
  return { lo: before.moved ? before.lo : before.hi + 1, hi: after.moved ? after.hi : after.lo - 1 };
}

/**
 * Whether `tip` (a file's lines) already holds a hunk where `moves` carries it: its new lines stand there in a run
 * (with other work merged beside them, maybe), or, for a hunk that only removes lines, they are gone from there.
 */
function heldIn(tip: readonly string[], hunk: RawHunk, moves: readonly RawHunk[]): boolean {
  const lines = (sign: string): string[] => hunk.lines.filter(line => line.startsWith(sign)).map(line => line.slice(1));
  const added = lines("+");
  const window = hunkWindow(hunk, moves);
  const there = tip.slice(Math.max(0, window.lo - 1), Math.max(0, window.hi));
  return added.length === 0 ? !containsRun(there, lines("-")) : containsRun(there, added);
}

/** At most `limit` of `lines`, with a count of the rest. */
export function clipLines(lines: readonly string[], limit: number): string {
  return lines.length <= limit ? lines.join("\n") : [...lines.slice(0, limit), `[${lines.length - limit} more lines]`].join("\n");
}

/** A raw hunk as a base range with its clipped text. */
function refHunk(hunk: RawHunk): RefHunk {
  return { ...rangeOf(hunk), text: clipLines(hunk.lines, HUNK_LINES) };
}

/** A file's lines as Git holds it at a commit, or null when the commit has no such file. */
type FileLines = readonly string[] | null;

/** The digest {@link contestedAnchor} gives `lines` of a file whose base text is `text`. */
function anchorOf(path: string, text: FileLines, lines: readonly { readonly lo: number; readonly hi: number }[]): string {
  const contested = lines.map(({ lo, hi }) => (text ?? []).slice(Math.max(0, Math.floor(lo) - 1), Math.floor(hi)).join(" ").replace(/\s+/gu, " ").trim());
  return createHash("sha256").update(JSON.stringify([path, contested])).digest("hex").slice(0, 24);
}

/** `path` at `revision`, as lines; null when the revision has no such file. A blob is read as it is, never textconv'd. */
async function fileLines(repo: string, revision: string, path: string): Promise<FileLines> {
  const text = await gitReadAsync(repo, ["cat-file", "blob", `${revision}:${path}`], { maxBuffer: 64 * 1024 * 1024 });
  if (text === undefined) return null;
  const lines = text.split("\n");
  if (lines.at(-1) === "") lines.pop();
  return lines;
}

/**
 * One detection pass's reads of other refs' work against this side's files (`paths`), each read once per pass: a
 * file's text at a commit, and a fork point's diff to this side's base (`start`) and to the base branch's tip
 * (`baseTip`), which refs forked at the same point share.
 */
export class OverlapReader {
  readonly #files = new Map<string, Promise<FileLines>>();
  readonly #drifts = new Map<string, Promise<Map<string, RawHunk[]>>>();
  readonly #blobs = new Map<string, Promise<string | null>>();

  constructor(
    private readonly repo: string,
    private readonly at: { readonly start: string; readonly baseTip: string },
    private readonly paths: ReadonlySet<string>,
  ) {}

  /** `path` at `revision`, read once. */
  #file(revision: string, path: string): Promise<FileLines> {
    const key = `${revision}:${path}`;
    let lines = this.#files.get(key);
    if (lines === undefined) {
      lines = fileLines(this.repo, revision, path);
      this.#files.set(key, lines);
    }
    return lines;
  }

  /** `from`'s diff to `to` in this side's files, read once; none when they are the same commit. */
  #drift(from: string, to: string): Promise<Map<string, RawHunk[]>> {
    if (from === to) return Promise.resolve(new Map());
    const key = `${from}..${to}`;
    let drift = this.#drifts.get(key);
    if (drift === undefined) {
      drift = diffHunks(this.repo, from, to, this.paths);
      this.#drifts.set(key, drift);
    }
    return drift;
  }

  /** The blob `path` names at `revision`, read once; null when it has none. */
  #blob(revision: string, path: string): Promise<string | null> {
    const key = `${revision}:${path}`;
    let blob = this.#blobs.get(key);
    if (blob === undefined) {
      blob = gitReadAsync(this.repo, ["rev-parse", "--verify", "--quiet", `${revision}:${path}`]).then(output => output?.trim() || null);
      this.#blobs.set(key, blob);
    }
    return blob;
  }

  /**
   * Where `sha`'s own work starts: its merge base with the base branch's tip, so mainline work it merged in is not its
   * own. Null when the base branch already holds all of `sha` (merged, or the base branch itself) or they share no
   * history.
   */
  async ownBase(sha: string): Promise<string | null> {
    const own = await mergeBaseAsync(this.repo, sha, this.at.baseTip);
    return own === sha ? null : own;
  }

  /**
   * Whether the base branch's tip already holds one hunk of `sha`'s own work in `path` (squash-merged, rebased or
   * cherry-picked in, the branch left behind): carried to the tip through `own`'s diff to it (`moves`), the tip's lines
   * between the hunk's unchanged neighbours are its new lines; or, where the tip changed a neighbour too, its new lines
   * stand there in a run. A binary change is held when the tip's file is the ref's.
   */
  async #inBaseTip(path: string, sha: string, hunk: RawHunk, moves: readonly RawHunk[]): Promise<boolean> {
    if (hunk.oldCount === WHOLE_FILE) {
      const [theirs, tip] = await Promise.all([this.#blob(sha, path), this.#blob(this.at.baseTip, path)]);
      return theirs === tip;
    }
    return heldIn((await this.#file(this.at.baseTip, path)) ?? [], hunk, moves);
  }

  /**
   * `sha`'s own changes to this side's files, from `own` (where its work starts: {@link ownBase}), as `start` lines:
   * each hunk's range in `own` carried to `start` through `own`'s diff to it, file by file. Work `sha` merged in from
   * its base branch after `own` is not its own, and work of its own the base branch's tip already holds is not live:
   * neither is counted.
   */
  async refHunks(own: string, sha: string): Promise<Map<string, RefHunk[]>> {
    const theirs = await diffHunks(this.repo, own, sha, this.paths);
    if (theirs.size === 0) return new Map();
    const [toStart, toTip] = await Promise.all([this.#drift(own, this.at.start), this.#drift(own, this.at.baseTip)]);
    const files = new Map<string, RefHunk[]>();
    for (const [path, hunks] of theirs) {
      const live: RefHunk[] = [];
      for (const hunk of hunks) {
        if (await this.#inBaseTip(path, sha, hunk, toTip.get(path) ?? [])) continue;
        const range = refHunk(hunk);
        live.push({ ...range, ...carryRange(range, toStart.get(path) ?? []) });
      }
      if (live.length > 0) files.set(path, live);
    }
    return files;
  }

  /** {@link contestedAnchor} at `start`, its file read once. */
  async anchor(path: string, lines: readonly { readonly lo: number; readonly hi: number }[]): Promise<string> {
    return anchorOf(path, await this.#file(this.at.start, path), lines);
  }
}

/** The working files' changes against `base`, untracked files as new files, as base ranges with their text. Reads only. */
export async function workspaceRanges(root: string, base: string): Promise<Map<string, RefHunk[]>> {
  const ranges = new Map<string, RefHunk[]>();
  for (const [path, hunks] of await diffText(root, base, null)) ranges.set(path, hunks.map(refHunk));
  const untracked = await gitReadAsync(root, ["ls-files", "--others", "--exclude-standard", "-z"], DIFF_OPTIONS) ?? "";
  for (const path of untracked.split("\0").filter(item => item !== "")) ranges.set(path, [{ lo: 0.5, hi: 0.5, sig: "untracked", text: "(new file)" }]);
  return ranges;
}

/**
 * What a change contests in `path`, anchored on content: a digest of the path and the text of the base lines each of
 * `lines` covers at `base` (an insertion: the line before it), whitespace collapsed. Lines added above them, a base
 * that moved without touching them, or a whitespace-only change keep it; a change to the text itself makes another. A
 * file `base` does not hold anchors on no text.
 */
export async function contestedAnchor(repo: string, base: string, path: string, lines: readonly { readonly lo: number; readonly hi: number }[]): Promise<string> {
  return anchorOf(path, await fileLines(repo, base, path), lines);
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
  refs: readonly { readonly ref: string; readonly files: ReadonlyMap<string, readonly RefHunk[]> }[],
): Array<GitConflict & { readonly ours: readonly Range[] }> {
  const conflicts: Array<GitConflict & { readonly ours: readonly Range[] }> = [];
  for (const side of refs) {
    for (const [file, mine] of agent) {
      const theirs = (side.files.get(file) ?? []).filter(hunk => mine.some(range => overlaps(range, hunk)));
      if (theirs.length === 0) continue;
      conflicts.push({ ref: side.ref, file, theirs, ours: mine.filter(range => theirs.some(hunk => overlaps(range, hunk))) });
    }
  }
  return conflicts;
}
