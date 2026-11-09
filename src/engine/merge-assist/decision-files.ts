/**
 * Decision ledger entries on disk and in Git, and the fingerprints of the positions they record. Entries are data
 * other people wrote, so they are read as candidates are (`guarded-files.ts`): no symlink followed, each size checked
 * before reading, at most {@link MAX_DECISION_ENTRIES_PER_TREE} by name order, only names {@link isDecisionFileName}
 * accepts. Writing puts one entry file in the working tree; Rerail never commits it.
 */
import { createHash } from "node:crypto";
import { readdir } from "node:fs/promises";
import { join } from "node:path";
import { readGitBlobs } from "../git.js";
import { CANDIDATE_DIRECTORY, candidateFileName, candidateHash, MAX_CANDIDATE_BYTES } from "./candidate.js";
import { candidateFromText } from "./candidate-files.js";
import {
  decisionEntryFromText,
  decisionFileName,
  DECISIONS_DIRECTORY,
  isDecisionFileName,
  MAX_DECISION_ENTRIES_PER_TREE,
  MAX_DECISION_ENTRY_BYTES,
  serializeDecisionEntry,
  type DecisionEntry,
  type Position,
} from "./decision-entry.js";
import { byteOrder, commitObject, readGuardedFile, realFolder, treeObjectEntries, unreadableEntry, writeGuardedFile } from "./guarded-files.js";
import { hunkLinesAt } from "./overlap.js";

export interface DecisionSet {
  readonly entries: DecisionEntry[];
  /** What could not be read, one per file or folder: `<file name>: <problem>`. */
  readonly problems: string[];
}

const RERAIL_DIRECTORY = DECISIONS_DIRECTORY.slice(0, DECISIONS_DIRECTORY.indexOf("/"));

/** The first {@link MAX_DECISION_ENTRIES_PER_TREE} names in byte order, and the problem that reports the rest. */
function capNames(names: readonly string[]): { read: string[]; problems: string[] } {
  const sorted = [...names].sort(byteOrder);
  const extra = sorted.length - MAX_DECISION_ENTRIES_PER_TREE;
  if (extra <= 0) return { read: sorted, problems: [] };
  return { read: sorted.slice(0, MAX_DECISION_ENTRIES_PER_TREE), problems: [`${DECISIONS_DIRECTORY}: ${extra} more entr${extra === 1 ? "y" : "ies"} not read; a tree holds at most ${MAX_DECISION_ENTRIES_PER_TREE}`] };
}

/** The entries in `root`'s working tree under `.rerail/decisions/`; a missing folder is an empty set. */
export async function readWorktreeDecisions(root: string): Promise<DecisionSet> {
  const problems: string[] = [];
  const folder = join(root, DECISIONS_DIRECTORY);
  if (!(await realFolder(join(root, RERAIL_DIRECTORY), RERAIL_DIRECTORY, problems)) || !(await realFolder(folder, DECISIONS_DIRECTORY, problems))) {
    return { entries: [], problems };
  }
  const { read, problems: extra } = capNames((await readdir(folder)).filter(isDecisionFileName));
  const entries: DecisionEntry[] = [];
  for (const name of read) {
    const file = await readGuardedFile(join(folder, name), name, MAX_DECISION_ENTRY_BYTES);
    const outcome = "text" in file ? decisionEntryFromText(name, file.text) : file;
    if ("entry" in outcome) entries.push(outcome.entry);
    else problems.push(outcome.problem);
  }
  return { entries, problems: [...problems, ...extra] };
}

/**
 * The entries in one `.rerail/decisions` tree object (`<commit>:.rerail/decisions`), read by one `git cat-file
 * --batch`. A tree never changes, so its id keys what was read from it.
 */
export async function readDecisionFolder(root: string, tree: string): Promise<DecisionSet> {
  const listed = treeObjectEntries(root, tree);
  if (listed === undefined) return { entries: [], problems: [`${DECISIONS_DIRECTORY}: Git could not list tree ${tree}`] };
  const named = new Map(listed.filter(entry => isDecisionFileName(entry.name)).map(entry => [entry.name, entry]));
  const { read, problems: extra } = capNames([...named.keys()]);
  const objects = read.map(name => named.get(name)!).filter(entry => unreadableEntry(entry, MAX_DECISION_ENTRY_BYTES) === undefined).map(entry => entry.object);
  const blobs = await readGitBlobs(root, objects, { maxBlobBytes: MAX_DECISION_ENTRY_BYTES });
  const entries: DecisionEntry[] = [];
  const problems: string[] = [];
  for (const name of read) {
    const listing = named.get(name)!;
    const problem = unreadableEntry(listing, MAX_DECISION_ENTRY_BYTES);
    const blob = blobs.get(listing.object);
    const outcome = problem !== undefined || blob === undefined ? { problem: `${name}: ${problem ?? "cannot be read from Git"}` } : decisionEntryFromText(name, blob.toString("utf8"));
    if ("entry" in outcome) entries.push(outcome.entry);
    else problems.push(outcome.problem);
  }
  return { entries, problems: [...problems, ...extra] };
}

/** Write `entry` to `root`'s `.rerail/decisions/<id>.json`, for the person to commit. Returns its repository path. */
export async function writeWorktreeDecision(root: string, entry: DecisionEntry): Promise<string> {
  const name = decisionFileName(entry.id);
  const text = serializeDecisionEntry(entry);
  const checked = decisionEntryFromText(name, text);
  if ("problem" in checked) throw new Error(`decision entry ${entry.id} was not written: ${checked.problem}`);
  await writeGuardedFile(root, DECISIONS_DIRECTORY, name, text);
  return `${DECISIONS_DIRECTORY}/${name}`;
}

/**
 * The fingerprint of hunk lines: sha256 over each `-`/`+` line with its whitespace collapsed, blank ones and Git's
 * `\ No newline` markers left out, so reindenting or a trailing space is no change of position.
 */
export function hunkFingerprint(lines: readonly string[]): string {
  const normalised = lines
    .filter(line => line.startsWith("+") || line.startsWith("-"))
    .map(line => `${line[0]}${line.slice(1).trim().replace(/\s+/gu, " ")}`)
    .filter(line => line.length > 1);
  return createHash("sha256").update(normalised.join("\n")).digest("hex");
}

/**
 * A side's fingerprint for `at` as of `to` (a commit, or null for `root`'s working files): its hunks against `base` at
 * the contested lines, or its candidate's content hash. Null when it cannot be read: a commit or base this repository
 * does not hold, or a candidate that is not there.
 */
export async function positionFingerprint(root: string, base: string, to: string | null, at: Position): Promise<string | null> {
  if (at.kind === "hunks") {
    if (commitObject(root, base) === undefined || (to !== null && commitObject(root, to) === undefined)) return null;
    try {
      return hunkFingerprint(hunkLinesAt(root, base, to, at.path, at.lines));
    } catch {
      return null;
    }
  }
  const name = candidateFileName(at.candidateId);
  let text: string | undefined;
  if (to === null) {
    const file = await readGuardedFile(join(root, CANDIDATE_DIRECTORY, name), name, MAX_CANDIDATE_BYTES);
    text = "text" in file ? file.text : undefined;
  } else {
    const object = commitObject(root, to);
    const revision = `${object}:${CANDIDATE_DIRECTORY}/${name}`;
    text = object === undefined ? undefined : (await readGitBlobs(root, [revision], { maxBlobBytes: MAX_CANDIDATE_BYTES })).get(revision)?.toString("utf8");
  }
  if (text === undefined) return null;
  const parsed = candidateFromText(name, text);
  return "candidate" in parsed ? candidateHash(parsed.candidate) : null;
}
