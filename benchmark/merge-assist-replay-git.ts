/**
 * The replay's `git-conflict` arms: a read-only identifier of the textual conflicts the agent's work would have with
 * other refs in flight. See benchmark/merge-assist-replay.ts.
 *
 * No merge is run and nothing is written to any repository. Both sides are `git diff --unified=0` hunks as line ranges
 * of the base: the agent's working files against the base, and another ref's work against its merge base with the
 * base, its ranges carried to the base's lines through the merge base's own diff to the base. Two hunks in the same
 * file conflict when they touch the same base lines or leave at most {@link ADJACENT_LINES} unchanged lines between
 * them, which approximates git's own conflict rule. Renames are ignored.
 */
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { fenceData, newJudgeNonce } from "../src/runtime/index.js";

/** The most unchanged base lines two hunks may leave between them and still conflict. */
export const ADJACENT_LINES = 3;
/** The most lines of another ref's change one item shows. */
export const HUNK_LINES = 40;
/** The most commit subjects a ref's stated intent quotes when it has no pull request title. */
const INTENT_SUBJECTS = 3;
/** A binary file's change covers the whole file. */
const WHOLE_FILE = 1e9;

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

/** One hunk of another ref's change: its base range and its text (the `@@` line and its `-`/`+` lines, clipped). */
export interface RefHunk extends LineRange {
  readonly text: string;
}

/** Another ref as visible when the work started, with its changes against the base. */
export interface RefSide {
  /** How the person knows it: `PR #12` or `branch a/b`. */
  readonly label: string;
  readonly ref: string;
  readonly sha: string;
  readonly author: string;
  /** When its first own commit became public. */
  readonly since: string;
  /** Its pull request title and the first paragraph of its body, or its latest commit subjects. */
  readonly intent: string;
  /** Its hunks per file, in base lines. */
  readonly files: Readonly<Record<string, readonly RefHunk[]>>;
}

/** One identified conflict: a ref, a file both sides change, and that ref's hunks the agent's overlap. */
export interface GitConflict {
  readonly ref: string;
  readonly file: string;
  readonly theirs: readonly RefHunk[];
}

const ENV: NodeJS.ProcessEnv = { ...process.env, GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: "/dev/null", GIT_OPTIONAL_LOCKS: "0", GIT_TERMINAL_PROMPT: "0" };

/** A read-only git command's output; null when it fails. */
export function gitRead(cwd: string, args: readonly string[]): string | null {
  const result = spawnSync("git", ["--no-optional-locks", "-c", "core.quotePath=false", ...args], { cwd, env: ENV, encoding: "utf8", maxBuffer: 512 * 1024 * 1024 });
  return result.status === 0 ? result.stdout : null;
}

/** A parsed hunk as a base range with its digest. */
function rangeOf(hunk: RawHunk): LineRange {
  const sig = createHash("sha1").update(hunk.lines.slice(1).join("\n")).digest("hex").slice(0, 16);
  const start = hunk.oldStart;
  return hunk.oldCount === 0 ? { lo: start + 0.5, hi: start + 0.5, sig } : { lo: start, hi: start + hunk.oldCount - 1, sig };
}

const HEADER = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/u;

/** One parsed `@@` hunk: both sides' starts and counts, and its text. */
interface RawHunk {
  readonly oldStart: number;
  readonly oldCount: number;
  readonly newStart: number;
  readonly newCount: number;
  readonly lines: string[];
}

function unquote(path: string): string {
  return path.startsWith("\"") && path.endsWith("\"") ? path.slice(1, -1) : path;
}

/** `git diff --unified=0 --no-renames` output as hunks per file (the post-image path, or the pre-image for a deletion). */
function parseDiff(text: string): Map<string, RawHunk[]> {
  const files = new Map<string, RawHunk[]>();
  let oldPath = "";
  let blobs = "";
  let current: RawHunk[] | null = null;
  let hunk: RawHunk | null = null;
  for (const line of text.split("\n")) {
    if (line.startsWith("diff --git ")) {
      current = null;
      hunk = null;
      oldPath = "";
      blobs = "";
    } else if (line.startsWith("index ") && hunk === null) {
      blobs = line;
    } else if (line.startsWith("--- ") && hunk === null) {
      oldPath = line === "--- /dev/null" ? "" : unquote(line.slice(4)).replace(/^a\//u, "");
    } else if (line.startsWith("+++ ") && hunk === null) {
      const path = line === "+++ /dev/null" ? oldPath : unquote(line.slice(4)).replace(/^b\//u, "");
      current = files.get(path) ?? [];
      files.set(path, current);
    } else if (line.startsWith("Binary files ")) {
      const match = /^Binary files (?:a\/(.+)|\/dev\/null) and (?:b\/(.+)|\/dev\/null) differ$/u.exec(line);
      const path = match?.[2] ?? match?.[1];
      // The blob ids tell two binary changes apart; a binary change covers the whole file.
      if (path !== undefined) files.set(path, [{ oldStart: 0, oldCount: WHOLE_FILE, newStart: 0, newCount: WHOLE_FILE, lines: ["(binary file changed)", blobs] }]);
    } else if (line.startsWith("@@ ") && current !== null) {
      const match = HEADER.exec(line);
      if (match === null) continue;
      hunk = { oldStart: Number(match[1]), oldCount: match[2] === undefined ? 1 : Number(match[2]), newStart: Number(match[3]), newCount: match[4] === undefined ? 1 : Number(match[4]), lines: [line] };
      current.push(hunk);
    } else if (hunk !== null && (line.startsWith("+") || line.startsWith("-") || line.startsWith("\\"))) {
      hunk.lines.push(line);
    }
  }
  return files;
}

/** `git diff --unified=0` between two commits (or a commit and the working files, `to` null), limited to `paths` when given. */
function diffHunks(repo: string, from: string, to: string | null, paths?: readonly string[]): Map<string, RawHunk[]> {
  if (paths !== undefined && paths.length === 0) return new Map();
  const args = ["diff", "--no-color", "--no-ext-diff", "--no-renames", "--unified=0", from, ...(to === null ? [] : [to])];
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

function clipLines(lines: readonly string[], limit: number): string {
  return lines.length <= limit ? lines.join("\n") : [...lines.slice(0, limit), `[${lines.length - limit} more lines]`].join("\n");
}

/**
 * `sha`'s changes against its merge base with `base`, as base lines: each hunk's merge-base range carried to the base
 * through the merge base's own diff to the base, file by file. `paths` limits the files read.
 */
export function refHunks(repo: string, base: string, sha: string, paths?: readonly string[]): Record<string, RefHunk[]> {
  const mergeBase = gitRead(repo, ["merge-base", base, sha])?.trim();
  if (mergeBase === undefined || mergeBase === "") return {};
  const theirs = diffHunks(repo, mergeBase, sha, paths);
  const drift = mergeBase === base || theirs.size === 0 ? new Map<string, RawHunk[]>() : diffHunks(repo, mergeBase, base, [...theirs.keys()]);
  const files: Record<string, RefHunk[]> = {};
  for (const [path, hunks] of theirs) {
    const moves = drift.get(path) ?? [];
    files[path] = hunks.map(hunk => {
      const range = rangeOf(hunk);
      const lo = moves.length === 0 ? range.lo : carry(range.lo, moves).lo;
      const hi = moves.length === 0 ? range.hi : carry(range.hi, moves).hi;
      return { lo, hi, sig: range.sig, text: clipLines(hunk.lines, HUNK_LINES) };
    });
  }
  return files;
}

/** The files `sha` changes against its merge base with `base`. */
export function refFiles(repo: string, base: string, sha: string): string[] {
  const mergeBase = gitRead(repo, ["merge-base", base, sha])?.trim();
  if (mergeBase === undefined || mergeBase === "") return [];
  return (gitRead(repo, ["diff", "--name-only", "--no-renames", mergeBase, sha]) ?? "").split("\n").filter(path => path !== "");
}

/** The agent's changes: the workspace's working files against `base`, untracked files as new files. Reads only. */
export function workspaceRanges(root: string, base: string): Map<string, LineRange[]> {
  const ranges = new Map<string, LineRange[]>();
  for (const [path, hunks] of diffHunks(root, base, null)) ranges.set(path, hunks.map(rangeOf));
  const untracked = gitRead(root, ["ls-files", "--others", "--exclude-standard", "-z"]) ?? "";
  for (const path of untracked.split("\0").filter(item => item !== "")) ranges.set(path, [{ lo: 0.5, hi: 0.5, sig: "untracked" }]);
  return ranges;
}

/** The agent's changes between two commits, limited to `paths`, as base ranges (`--detect-only`). */
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

/** Every (ref, file) where one of the agent's hunks overlaps one of the ref's, with the ref's overlapping hunks. */
export function identify(agent: ReadonlyMap<string, readonly LineRange[]>, refs: readonly Pick<RefSide, "ref" | "files">[]): GitConflict[] {
  const conflicts: GitConflict[] = [];
  for (const side of refs) {
    for (const [file, ours] of agent) {
      const theirs = (side.files[file] ?? []).filter(hunk => ours.some(range => overlaps(range, hunk)));
      if (theirs.length > 0) conflicts.push({ ref: side.ref, file, theirs });
    }
  }
  return conflicts;
}

/** A conflict's key in hook state and logs. */
export function conflictKey(conflict: Pick<GitConflict, "ref" | "file">): string {
  return `${conflict.ref}\u0000${conflict.file}`;
}

/** A ref's stated intent: its pull request title and first body paragraph, else its latest commit subjects. */
export function statedIntent(repo: string, base: string, sha: string, pr: { title: string; body?: string } | null): string {
  if (pr !== null) {
    const paragraph = (pr.body ?? "").trim().split(/\n\s*\n/u)[0]?.trim() ?? "";
    return paragraph === "" ? pr.title : `${pr.title}\n${paragraph}`;
  }
  const mergeBase = gitRead(repo, ["merge-base", base, sha])?.trim() ?? "";
  const range = mergeBase === "" ? sha : `${mergeBase}..${sha}`;
  return (gitRead(repo, ["log", "--no-merges", `-${INTENT_SUBJECTS}`, "--format=%s", range]) ?? "").trim();
}

/** Their overlapping hunks as one change, at most {@link HUNK_LINES} lines. */
function theirChange(conflict: GitConflict): string {
  return clipLines(conflict.theirs.flatMap(hunk => hunk.text.split("\n")), HUNK_LINES);
}

function sideOf(conflict: GitConflict, refs: readonly RefSide[]): RefSide {
  const side = refs.find(candidate => candidate.ref === conflict.ref);
  if (side === undefined) throw new Error(`no ref ${conflict.ref}`);
  return side;
}

/** `git-conflict`'s text for one conflict: who, where, their change and stated intent fenced as data, then what to do. */
export function pushText(conflict: GitConflict, refs: readonly RefSide[], nonce: string = newJudgeNonce()): string {
  const side = sideOf(conflict, refs);
  return [
    `Git conflict in flight: ${side.label} by ${side.author} (public since ${side.since}) also changes ${conflict.file}; your changes would conflict with theirs. Their change:`,
    fenceData("their change", theirChange(conflict), nonce),
    "Their stated intent:",
    fenceData("their stated intent", side.intent, nonce),
    "Make your code compatible with theirs, unless that is contrary to the approach your task asks for; if it is, keep yours and tell the person in one line which branch you diverge from and why.",
  ].join("\n");
}

/** `git-conflict-pull`'s one line, sent when the set of conflicts changes. */
export function pullLine(conflicts: readonly GitConflict[]): string {
  if (conflicts.length === 0) return "In flight: your changes no longer conflict with other branches (`git conflicts` lists none).";
  const refs = new Set(conflicts.map(conflict => conflict.ref)).size;
  const files = [...new Set(conflicts.map(conflict => conflict.file))];
  return `In flight: your changes would conflict with ${refs} other branch${refs === 1 ? "" : "es"} (${files.join(", ")}). See them with \`git conflicts\`. Decide when to deal with it; before you finish, make your code compatible with theirs unless that is contrary to the approach your task asks for, and tell the person which you diverged from.`;
}

/** What `git conflicts` prints under `git-conflict-pull`: one item per (ref, file), data fenced. */
export function pullFile(conflicts: readonly GitConflict[], refs: readonly RefSide[], nonce: string = newJudgeNonce()): string {
  if (conflicts.length === 0) return "No in-flight conflicts: none of your changes overlap another branch's.\n";
  const items = conflicts.map((conflict, index) => {
    const side = sideOf(conflict, refs);
    return [
      `## ${index + 1}. ${side.label} by ${side.author} (public since ${side.since}): ${conflict.file}`,
      "",
      "Their change:",
      fenceData("their change", theirChange(conflict), nonce),
      "",
      "Their stated intent:",
      fenceData("their stated intent", side.intent, nonce),
    ].join("\n");
  });
  return [
    "# In-flight conflicts",
    "",
    `Your changes would conflict with ${conflicts.length === 1 ? "this change" : `these ${conflicts.length} changes`} on other branches. Before you finish, make your code compatible with theirs unless that is contrary to the approach your task asks for, and tell the person which you diverged from.`,
    "",
    items.join("\n\n"),
    "",
  ].join("\n");
}
