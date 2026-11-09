/**
 * Candidate files on disk and in Git. Candidates are data other people wrote, so reading them follows no symlink
 * (not `.rerail`, not `.rerail/candidates`, not a file), checks each size before reading, reads at most
 * {@link MAX_CANDIDATES_PER_BRANCH} files by name order, and only names {@link isCandidateFileName} accepts; other
 * names in the folder are left alone. A Git tree is read by object, from regular-file blobs only (`guarded-files.ts`).
 */
import { createHash } from "node:crypto";
import { closeSync, constants, fstatSync, lstatSync, openSync, readSync } from "node:fs";
import { readdir } from "node:fs/promises";
import { join } from "node:path";
import { readGitBlobs, runGit } from "../git.js";
import {
  CANDIDATE_DIRECTORY,
  candidateFileName,
  isCandidateFileName,
  MAX_CANDIDATE_BYTES,
  MAX_CANDIDATES_PER_BRANCH,
  parseCandidateText,
  serializeCandidate,
  type Candidate,
} from "./candidate.js";
import { byteOrder, commitObject, NO_FOLLOW, NOT_REGULAR, readGuardedFile, realFolder, tooLarge, treeFolderEntries, unreadableEntry, writeGuardedFile } from "./guarded-files.js";

export interface CandidateSet {
  readonly candidates: Candidate[];
  /** What could not be read, one per file or folder: `<file name>: <problem>`. */
  readonly problems: string[];
  /** Whether more candidate files were there than are read. */
  readonly truncated: boolean;
}

const RERAIL_DIRECTORY = CANDIDATE_DIRECTORY.slice(0, CANDIDATE_DIRECTORY.indexOf("/"));
/** The problem for a candidate file over the size limit, after `<file name>: `. */
export const CANDIDATE_TOO_LARGE = tooLarge(MAX_CANDIDATE_BYTES);
/** The problem for a candidate name that is a folder, symlink or other non-regular entry, after `<file name>: `. */
export const CANDIDATE_NOT_REGULAR = NOT_REGULAR;

/**
 * The candidate file names to read: the first {@link MAX_CANDIDATES_PER_BRANCH} in byte order, so every machine and
 * source reads the same ones, and the one problem that reports the rest.
 */
export function capCandidateNames(names: readonly string[]): { read: string[]; problems: string[]; truncated: boolean } {
  const sorted = [...names].sort(byteOrder);
  const extra = sorted.length - MAX_CANDIDATES_PER_BRANCH;
  if (extra <= 0) return { read: sorted, problems: [], truncated: false };
  return {
    read: sorted.slice(0, MAX_CANDIDATES_PER_BRANCH),
    problems: [`${CANDIDATE_DIRECTORY}: ${extra} more candidate file${extra === 1 ? "" : "s"} not read; a branch holds at most ${MAX_CANDIDATES_PER_BRANCH}`],
    truncated: true,
  };
}

/** One file's text as a candidate, or the problem with it; the file's name must be the one its id gives. */
export function candidateFromText(name: string, text: string): { candidate: Candidate } | { problem: string } {
  const parsed = parseCandidateText(text);
  if (!parsed.ok) return { problem: `${name}: ${parsed.problem}` };
  if (candidateFileName(parsed.candidate.id) !== name) return { problem: `${name}: its id ${parsed.candidate.id} belongs in ${candidateFileName(parsed.candidate.id)}` };
  return { candidate: parsed.candidate };
}

/**
 * The candidates in `root`'s working tree, under `.rerail/candidates/`: regular files only (never through a symlink,
 * `.rerail` and the folder included), names matching {@link isCandidateFileName}, each size checked before reading, at
 * most {@link MAX_CANDIDATES_PER_BRANCH} in name order. A missing folder is an empty set.
 */
export async function readWorktreeCandidates(root: string): Promise<CandidateSet> {
  const problems: string[] = [];
  const folder = join(root, CANDIDATE_DIRECTORY);
  if (!(await realFolder(join(root, RERAIL_DIRECTORY), RERAIL_DIRECTORY, problems)) || !(await realFolder(folder, CANDIDATE_DIRECTORY, problems))) {
    return { candidates: [], problems, truncated: false };
  }
  const names = (await readdir(folder)).filter(isCandidateFileName);
  const { read, problems: extra, truncated } = capCandidateNames(names);
  const candidates: Candidate[] = [];
  for (const name of read) {
    const file = await readGuardedFile(join(folder, name), name, MAX_CANDIDATE_BYTES);
    const outcome = "text" in file ? candidateFromText(name, file.text) : file;
    if ("candidate" in outcome) candidates.push(outcome.candidate);
    else problems.push(outcome.problem);
  }
  return { candidates, problems: [...problems, ...extra], truncated };
}

/**
 * Write `candidate` to `root`'s `.rerail/candidates/<name>.json`, mode 0o644 so it can be committed (`writeGuardedFile`).
 * Returns the file's absolute path.
 */
export async function writeWorktreeCandidate(root: string, candidate: Candidate): Promise<string> {
  const name = candidateFileName(candidate.id);
  const text = serializeCandidate(candidate);
  const checked = candidateFromText(name, text);
  if ("problem" in checked) throw new Error(`candidate ${candidate.id} was not written: ${checked.problem}`);
  return await writeGuardedFile(root, CANDIDATE_DIRECTORY, name, text);
}

/**
 * The candidates in a commit's tree under `.rerail/candidates/`: regular-file blobs (mode 100644) only, names matching
 * {@link isCandidateFileName}, each size checked from the listing before the blob is read, at most
 * {@link MAX_CANDIDATES_PER_BRANCH} in name order, all read by one `git cat-file --batch`. A commit this repository does
 * not hold is an empty set with a problem.
 */
export async function readTreeCandidates(repoRoot: string, commit: string): Promise<CandidateSet> {
  const object = commitObject(repoRoot, commit);
  if (object === undefined) return { candidates: [], problems: [`${commit}: not a commit in this repository`], truncated: false };
  const entries = treeFolderEntries(repoRoot, object, CANDIDATE_DIRECTORY);
  if (entries === undefined) return { candidates: [], problems: [`${CANDIDATE_DIRECTORY}: Git could not list it at ${commit}`], truncated: false };
  const named = new Map(entries.filter(entry => isCandidateFileName(entry.name)).map(entry => [entry.name, entry]));
  const { read, problems: extra, truncated } = capCandidateNames([...named.keys()]);
  const objects = read.map(name => named.get(name)!).filter(entry => unreadableEntry(entry, MAX_CANDIDATE_BYTES) === undefined).map(entry => entry.object);
  const blobs = await readGitBlobs(repoRoot, objects, { maxBlobBytes: MAX_CANDIDATE_BYTES });
  const candidates: Candidate[] = [];
  const problems: string[] = [];
  for (const name of read) {
    const entry = named.get(name)!;
    const problem = unreadableEntry(entry, MAX_CANDIDATE_BYTES);
    const blob = blobs.get(entry.object);
    if (problem !== undefined || blob === undefined) {
      problems.push(`${name}: ${problem ?? "cannot be read from Git"}`);
      continue;
    }
    const parsed = candidateFromText(name, blob.toString("utf8"));
    if ("candidate" in parsed) candidates.push(parsed.candidate);
    else problems.push(parsed.problem);
  }
  return { candidates, problems: [...problems, ...extra], truncated };
}

/** Whether Git ignores `.rerail/candidates/` in this checkout, so candidates written there stay local. */
export function candidatesIgnored(root: string): boolean {
  return runGit(root, ["check-ignore", "--quiet", "--", `${CANDIDATE_DIRECTORY}/${candidateFileName("candidate:rerail-probe")}`]).status === 0;
}

/** A working file's bytes, read without following a symlink and only when it is a regular file within the size limit. */
function workingFileBytes(path: string): Buffer | undefined {
  let descriptor: number;
  try {
    if (!lstatSync(path).isFile()) return undefined;
    descriptor = openSync(path, constants.O_RDONLY | NO_FOLLOW);
  } catch {
    return undefined;
  }
  try {
    const entry = fstatSync(descriptor);
    if (!entry.isFile() || entry.size > MAX_CANDIDATE_BYTES) return undefined;
    const buffer = Buffer.alloc(MAX_CANDIDATE_BYTES + 1);
    let length = 0;
    for (;;) {
      const read = readSync(descriptor, buffer, length, buffer.length - length, null);
      if (read === 0) break;
      length += read;
      if (length > MAX_CANDIDATE_BYTES) return undefined;
    }
    return buffer.subarray(0, length);
  } finally {
    closeSync(descriptor);
  }
}

/** Git's object id for `bytes` as a blob, in the repository's object format. */
function blobObjectId(bytes: Buffer, format: string): string {
  return createHash(format).update(`blob ${bytes.length}\0`).update(bytes).digest("hex");
}

/**
 * The ids among `candidateIds` whose working file is byte for byte the regular file in HEAD's tree, so pushing shares
 * it. HEAD's candidate folder is listed once for all of them.
 */
export function committedCandidates(root: string, candidateIds: readonly string[]): Set<string> {
  const committed = new Set<string>();
  if (candidateIds.length === 0) return committed;
  const head = commitObject(root, "HEAD");
  if (head === undefined) return committed;
  const entries = new Map((treeFolderEntries(root, head, CANDIDATE_DIRECTORY) ?? []).map(entry => [entry.name, entry]));
  const format = runGit(root, ["rev-parse", "--show-object-format"]).stdout.trim() || "sha1";
  if (format !== "sha1" && format !== "sha256") return committed;
  for (const id of candidateIds) {
    const name = candidateFileName(id);
    const entry = entries.get(name);
    if (entry === undefined || entry.type !== "blob" || entry.mode !== "100644") continue;
    const bytes = workingFileBytes(join(root, CANDIDATE_DIRECTORY, name));
    if (bytes !== undefined && bytes.length === entry.size && blobObjectId(bytes, format) === entry.object) committed.add(id);
  }
  return committed;
}
