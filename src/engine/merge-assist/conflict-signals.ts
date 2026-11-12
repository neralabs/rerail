/**
 * The dimmer's signals for one open conflict, measured from Git (docs/merge-assist-v1-build.md §9.1): how hard the
 * contested change is to undo (lock-in), how much depends on it (blast radius), how fast both sides build on it
 * (growth), and how long until it meets the other side at merge. Reads only; merges and writes nothing.
 */
import { existsSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { runGit } from "../git.js";
import { gitRead } from "./branch-git.js";
import { quietFiles } from "./conflicts.js";
import type { ConflictSignals, LockInClass } from "./model.js";

/** The most files in the base that count as callers. */
export const MAX_CALLERS = 200;
/** Time to merge when theirs has a pull request open, and the least time to merge otherwise. */
export const PULL_REQUEST_HOURS = 24;
/** The longest time to merge the dimmer assumes: two weeks. */
export const MAX_HOURS_TO_MERGE = 336;
/** P(this side loses) when nothing says otherwise. */
export const EVEN_ODDS = 0.5;

const HOUR_MS = 3_600_000;
const MAX_SYMBOLS = 16;
const MAX_UNTRACKED_BYTES = 5 * 1024 * 1024;
const GIT_OPTIONS = { maxBuffer: 64 * 1024 * 1024, timeout: 60_000 };

/** Persisted schema, migrations and file formats: migrations, SQL, `schema.*`, protobuf and OpenAPI files. */
const PERSISTED: readonly RegExp[] = [
  /(?:^|\/)migrations?\//iu,
  /\.sql$/iu,
  /(?:^|\/)schema\.[^/]+$/iu,
  /\.proto$/iu,
  /(?:^|\/)(?:openapi|swagger)[^/]*\.(?:ya?ml|json)$/iu,
];
/** A package's entry or index file, where an exported signature is its public API. */
const ENTRY_FILE = /(?:^|\/)(?:index|main|lib|mod|__init__)\.[a-z]+$/iu;
/** A changed line declaring an exported symbol, and its name. */
const EXPORTED = new RegExp([
  String.raw`^[+-]\s*export\s+(?:default\s+)?(?:declare\s+)?(?:abstract\s+)?(?:async\s+)?(?:function\*?|class|const|let|var|interface|type|enum)\s+([A-Za-z_$][\w$]*)`,
  String.raw`^[+-]\s*pub(?:\([^)]*\))?\s+(?:async\s+)?(?:fn|struct|enum|trait|type|const|static|mod)\s+([A-Za-z_]\w*)`,
  String.raw`^[+-]\s*func\s+(?:\([^)]*\)\s*)?([A-Z]\w*)`,
  String.raw`^[+-]def\s+([A-Za-z]\w*)`,
  String.raw`^[+-]\s*public\s+(?:static\s+)?(?:final\s+)?[\w<>[\],.?\s]+?\s([A-Za-z_]\w*)\s*\(`,
].join("|"), "u");
/** A declaration's name, on a changed line or a hunk header. */
const DECLARED = /\b(?:function\*?|class|interface|type|enum|const|let|var|def|fn|func|struct|trait)\s+(?:\([^)]*\)\s*)?([A-Za-z_$][\w$]*)/gu;
const NOT_NAMES: Readonly<Record<string, true>> = {
  async: true, await: true, const: true, default: true, function: true, let: true, new: true, return: true, this: true, type: true, var: true, void: true,
};

/** A conflict as the dimmer measures it. */
export interface ContestedChange {
  /** The contested files: the overlap's file. */
  readonly files: readonly string[];
  /** This branch's and theirs' contested hunks, as unified diff text clipped for display: their symbols count as callers. */
  readonly hunks: { readonly mine: string | null; readonly theirs: string | null };
  /** Both sides' full changed lines at the contested lines, when Git can diff them: what lock-in reads exports from. */
  readonly changed: { readonly mine: string | null; readonly theirs: string | null } | null;
  /** Their tip, and where their own work starts (null when the base holds all of it). */
  readonly theirs: { readonly sha: string; readonly own: string | null };
  readonly pullRequest: boolean;
  /** Other live refs that touch the contested files. */
  readonly otherRefs: number;
}

/** Where this checkout stands: the base branch's tip, where this branch's work starts, and its head. */
export interface SignalCheckout {
  readonly root: string;
  readonly baseRef: string;
  readonly start: string;
  readonly head: string;
  readonly now: number;
}

/** Exported names a side's hunk declares on its removed lines and on its added lines. */
function exportedNames(hunk: string | null): { removed: Set<string>; added: Set<string> } {
  const removed = new Set<string>();
  const added = new Set<string>();
  for (const line of (hunk ?? "").split("\n")) {
    const match = EXPORTED.exec(line);
    const name = match?.slice(1).find(group => group !== undefined);
    if (name !== undefined) (line.startsWith("-") ? removed : added).add(name);
  }
  return { removed, added };
}

/**
 * Whether a side deleted a contested file it started with: this checkout's working files no longer have one its start
 * had, or their tip no longer has one their own work started with.
 */
function deletedFile(checkout: SignalCheckout, change: ContestedChange): boolean {
  const has = (revision: string, file: string): boolean => runGit(checkout.root, ["cat-file", "-e", `${revision}:${file}`]).status === 0;
  const { own, sha } = change.theirs;
  return change.files.some(file => (has(checkout.start, file) && !existsSync(join(checkout.root, file)))
    || (own !== null && has(own, file) && !has(sha, file)));
}

/** How hard the contested change is to undo, and why. */
export function lockInOf(checkout: SignalCheckout, change: ContestedChange): { lockIn: 1 | 2 | 4; lockInClass: LockInClass } {
  if (change.files.some(file => PERSISTED.some(pattern => pattern.test(file)))) return { lockIn: 4, lockInClass: "persisted" };
  const lines = change.changed ?? change.hunks;
  const sides = [exportedNames(lines.mine), exportedNames(lines.theirs)];
  const signatures = sides.some(side => side.removed.size > 0 || side.added.size > 0);
  if (signatures && change.files.some(file => ENTRY_FILE.test(file))) return { lockIn: 4, lockInClass: "public-api" };
  const retired = sides.some(side => [...side.removed].some(name => !side.added.has(name)));
  if (retired || deletedFile(checkout, change)) return { lockIn: 4, lockInClass: "retirement" };
  return signatures ? { lockIn: 2, lockInClass: "exported" } : { lockIn: 1, lockInClass: "local" };
}

/** The symbols both sides' contested hunks declare or change, the hunk headers' included. */
export function contestedSymbols(hunks: ContestedChange["hunks"]): string[] {
  const names = new Set<string>();
  for (const line of [hunks.mine, hunks.theirs].flatMap(hunk => (hunk ?? "").split("\n"))) {
    if (!/^(?:[+-]|@@)/u.test(line)) continue;
    for (const match of line.matchAll(DECLARED)) {
      const name = match[1]!;
      if (name.length >= 3 && NOT_NAMES[name] !== true) names.add(name);
    }
  }
  return [...names].slice(0, MAX_SYMBOLS);
}

/** Files in the base, other than the contested ones, that name one of `symbols` as a whole word; at most {@link MAX_CALLERS}. */
export function callersOf(checkout: SignalCheckout, symbols: readonly string[], contested: readonly string[]): number {
  if (symbols.length === 0) return 0;
  const output = gitRead(checkout.root, ["grep", "-l", "-w", "-F", ...symbols.flatMap(symbol => ["-e", symbol]), checkout.baseRef, "--"], GIT_OPTIONS) ?? "";
  const prefix = `${checkout.baseRef}:`;
  const skip = new Set(contested);
  const files = output.split("\n").filter(line => line !== "").map(line => (line.startsWith(prefix) ? line.slice(prefix.length) : line));
  return Math.min(MAX_CALLERS, files.filter(file => !skip.has(file)).length);
}

/** Lines added by the commits in `range` to `specs`. */
function linesAdded(root: string, range: string, specs: readonly string[]): number {
  return sumAdded(gitRead(root, ["log", "--no-renames", "--numstat", "--format=", range, "--", ...specs], GIT_OPTIONS) ?? "");
}

/** The first column of `--numstat` output, binary files left out. */
function sumAdded(numstat: string): number {
  let total = 0;
  for (const line of numstat.split("\n")) {
    const added = Number(line.split("\t")[0]);
    if (line !== "" && Number.isFinite(added)) total += added;
  }
  return total;
}

/** Lines this checkout's working files add to `specs` against HEAD, untracked files counted whole. */
function workingLines(root: string, specs: readonly string[]): number {
  const tracked = sumAdded(gitRead(root, ["diff", "--no-renames", "--numstat", "HEAD", "--", ...specs], GIT_OPTIONS) ?? "");
  const untracked = (gitRead(root, ["ls-files", "--others", "--exclude-standard", "-z", "--", ...specs], GIT_OPTIONS) ?? "").split("\0").filter(path => path !== "");
  let lines = 0;
  for (const path of untracked) {
    try {
      const file = join(root, path);
      if (statSync(file).size <= MAX_UNTRACKED_BYTES) lines += readFileSync(file, "utf8").split("\n").filter(line => line !== "").length;
    } catch {
      // A file removed meanwhile adds nothing.
    }
  }
  return tracked + lines;
}

/** A commit's committer time in ms; null when it cannot be read. */
function commitTime(root: string, sha: string): number | null {
  const seconds = Number((gitRead(root, ["log", "-1", "--format=%ct", sha, "--"], GIT_OPTIONS) ?? "").trim());
  return Number.isFinite(seconds) && seconds > 0 ? seconds * 1_000 : null;
}

/** Hours until theirs merges: a day with a pull request open, else its age, at least a day and at most two weeks. */
function hoursToMergeOf(checkout: SignalCheckout, change: ContestedChange): number {
  if (change.pullRequest || change.theirs.own === null) return PULL_REQUEST_HOURS;
  const times = (gitRead(checkout.root, ["log", "--format=%ct", `${change.theirs.own}..${change.theirs.sha}`, "--"], GIT_OPTIONS) ?? "")
    .split("\n").map(Number).filter(seconds => Number.isFinite(seconds) && seconds > 0);
  const age = times.length === 0 ? 0 : (checkout.now - Math.min(...times) * 1_000) / HOUR_MS;
  return Math.min(MAX_HOURS_TO_MERGE, Math.max(PULL_REQUEST_HOURS, age));
}

/**
 * Every signal the dimmer reads for one open conflict. Growth is a stable rate: every line both sides added to the
 * contested files (this branch's committed and working lines together) over the hours since their work forked, so the
 * same lines score the same on every pass and a quiet conflict's rate only falls as time passes.
 */
export function measureSignals(checkout: SignalCheckout, change: ContestedChange): ConflictSignals {
  const { root, start, head, now } = checkout;
  const specs = change.files.map(file => `:(literal)${file}`);
  const fork = Math.min(...[start, change.theirs.own ?? start].map(sha => commitTime(root, sha) ?? now));
  const windowHours = Math.max(1, (now - fork) / HOUR_MS);
  const linesTheirs = change.theirs.own === null ? 0 : linesAdded(root, `${change.theirs.own}..${change.theirs.sha}`, specs);
  const linesMine = linesAdded(root, `${start}..${head}`, specs) + workingLines(root, specs);
  const callers = callersOf(checkout, contestedSymbols(change.hunks), change.files);
  return {
    quiet: quietFiles(change.files),
    ...lockInOf(checkout, change),
    blastRadius: callers + change.otherRefs,
    callers,
    otherRefs: change.otherRefs,
    growthPerHour: (linesTheirs + linesMine) / windowHours,
    linesTheirs,
    linesMine,
    windowHours,
    pLose: EVEN_ODDS,
    hoursToMerge: hoursToMergeOf(checkout, change),
    pullRequest: change.pullRequest,
  };
}
