/**
 * Candidate files on disk and in Git. Candidates are data other people wrote, so reading them follows no symlink
 * (not `.rerail`, not `.rerail/candidates`, not a file), checks each size before reading, reads at most
 * {@link MAX_CANDIDATES_PER_BRANCH} files by name order, and only names {@link isCandidateFileName} accepts; other
 * names in the folder are left alone. A Git tree is read by object, from regular-file blobs only.
 */
import { createHash, randomBytes } from "node:crypto";
import { closeSync, constants, fstatSync, lstatSync, openSync, readSync } from "node:fs";
import { chmod, lstat, mkdir, open, readdir, rename, unlink } from "node:fs/promises";
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

export interface CandidateSet {
  readonly candidates: Candidate[];
  /** What could not be read, one per file or folder: `<file name>: <problem>`. */
  readonly problems: string[];
  /** Whether more candidate files were there than are read. */
  readonly truncated: boolean;
}

/** `O_NOFOLLOW` where the platform has it; Windows has no symlink-following open to refuse. */
const NO_FOLLOW = constants.O_NOFOLLOW ?? 0;
const RERAIL_DIRECTORY = CANDIDATE_DIRECTORY.slice(0, CANDIDATE_DIRECTORY.indexOf("/"));
/** The problem for a candidate file over the size limit, after `<file name>: `. */
export const CANDIDATE_TOO_LARGE = `larger than ${MAX_CANDIDATE_BYTES} bytes`;
/** The problem for a candidate name that is a folder, symlink or other non-regular entry, after `<file name>: `. */
export const CANDIDATE_NOT_REGULAR = "not a regular file";

function errorCode(error: unknown): string | undefined {
  return typeof error === "object" && error !== null && "code" in error ? String(error.code) : undefined;
}

/** Names in byte order, so every machine reads the same first {@link MAX_CANDIDATES_PER_BRANCH}. */
function byteOrder(left: string, right: string): number {
  return Buffer.compare(Buffer.from(left), Buffer.from(right));
}

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
 * Whether the folder at `path` (relative name `label`) is a real directory: false with no problem when it is missing,
 * false with a problem when it is anything else, a symlink included.
 */
async function realFolder(path: string, label: string, problems: string[]): Promise<boolean> {
  try {
    const entry = await lstat(path);
    if (entry.isDirectory()) return true;
    problems.push(`${label}: not a folder${entry.isSymbolicLink() ? " (a symbolic link, which Rerail does not follow)" : ""}`);
    return false;
  } catch (error) {
    if (errorCode(error) === "ENOENT") return false;
    problems.push(`${label}: cannot be read (${errorCode(error) ?? "error"})`);
    return false;
  }
}

/** A candidate file's bytes, opened without following a symlink, its size checked on the open file before reading. */
async function readCandidateFile(path: string, name: string): Promise<{ text: string } | { problem: string }> {
  let handle;
  try {
    handle = await open(path, constants.O_RDONLY | NO_FOLLOW);
  } catch (error) {
    return { problem: `${name}: ${errorCode(error) === "ELOOP" ? CANDIDATE_NOT_REGULAR : `cannot be read (${errorCode(error) ?? "error"})`}` };
  }
  try {
    const entry = await handle.stat();
    if (!entry.isFile()) return { problem: `${name}: ${CANDIDATE_NOT_REGULAR}` };
    if (entry.size > MAX_CANDIDATE_BYTES) return { problem: `${name}: ${CANDIDATE_TOO_LARGE}` };
    // One byte more than the limit, so a file that grew since the check is still caught.
    const buffer = Buffer.alloc(MAX_CANDIDATE_BYTES + 1);
    let length = 0;
    for (;;) {
      const { bytesRead } = await handle.read(buffer, length, buffer.length - length, null);
      if (bytesRead === 0) break;
      length += bytesRead;
      if (length > MAX_CANDIDATE_BYTES) return { problem: `${name}: ${CANDIDATE_TOO_LARGE}` };
    }
    return { text: buffer.subarray(0, length).toString("utf8") };
  } finally {
    await handle.close();
  }
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
    const file = await readCandidateFile(join(folder, name), name);
    const outcome = "text" in file ? candidateFromText(name, file.text) : file;
    if ("candidate" in outcome) candidates.push(outcome.candidate);
    else problems.push(outcome.problem);
  }
  return { candidates, problems: [...problems, ...extra], truncated };
}

/** Create the folder at `path` (0o755) unless it is there; refuse anything there that is not a real directory. */
async function ensureFolder(path: string, label: string): Promise<void> {
  try {
    await mkdir(path, { mode: 0o755 });
  } catch (error) {
    if (errorCode(error) !== "EEXIST") throw error;
  }
  const entry = await lstat(path);
  if (!entry.isDirectory()) throw new Error(`${label} is not a folder${entry.isSymbolicLink() ? " (a symbolic link, which Rerail does not follow)" : ""}, so the candidate was not written`);
}

/**
 * Write `candidate` to `root`'s `.rerail/candidates/<name>.json`, mode 0o644 so it can be committed: the folders are
 * created as needed and refused when either is a symlink, the text goes to a new temporary file opened exclusively
 * without following a symlink, and a rename puts it in place. Returns the file's absolute path.
 */
export async function writeWorktreeCandidate(root: string, candidate: Candidate): Promise<string> {
  const name = candidateFileName(candidate.id);
  const text = serializeCandidate(candidate);
  const checked = candidateFromText(name, text);
  if ("problem" in checked) throw new Error(`candidate ${candidate.id} was not written: ${checked.problem}`);
  await ensureFolder(join(root, RERAIL_DIRECTORY), RERAIL_DIRECTORY);
  const folder = join(root, CANDIDATE_DIRECTORY);
  await ensureFolder(folder, CANDIDATE_DIRECTORY);
  const path = join(folder, name);
  const temporary = join(folder, `.${name}.${randomBytes(6).toString("hex")}.tmp`);
  const handle = await open(temporary, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | NO_FOLLOW, 0o644);
  try {
    try {
      await handle.writeFile(text, "utf8");
      await handle.sync();
    } finally {
      await handle.close();
    }
    // The mode a umask narrowed is set back: the file is meant to be committed and read by anyone.
    await chmod(temporary, 0o644);
    await rename(temporary, path);
  } catch (error) {
    await unlink(temporary).catch(() => undefined);
    throw error;
  }
  return path;
}

/** One `git ls-tree -l -z` entry under the candidate folder. */
interface TreeEntry {
  readonly mode: string;
  readonly type: string;
  readonly object: string;
  readonly size: number | null;
  readonly name: string;
}

/** The entries of `git ls-tree -l -z <commit> -- .rerail/candidates/` directly in that folder, or undefined when Git fails. */
function treeEntries(repoRoot: string, commit: string): TreeEntry[] | undefined {
  const result = runGit(repoRoot, ["ls-tree", "-l", "-z", "--full-tree", commit, "--", `${CANDIDATE_DIRECTORY}/`], { maxBuffer: 16 * 1024 * 1024 });
  if (result.status !== 0 || result.error !== undefined) return undefined;
  const prefix = `${CANDIDATE_DIRECTORY}/`;
  return result.stdout.split("\0").flatMap(record => {
    // `<mode> SP <type> SP <object> SP+ <size> TAB <path>`; the size is `-` for a tree.
    const match = /^(\d{6}) (\w+) ([0-9a-f]+) +(\d+|-)\t(.+)$/su.exec(record);
    if (match === null) return [];
    const [, mode = "", type = "", object = "", size = "", path = ""] = match;
    if (!path.startsWith(prefix) || path.slice(prefix.length).includes("/")) return [];
    return [{ mode, type, object, size: size === "-" ? null : Number(size), name: path.slice(prefix.length) }];
  });
}

/** A commit's object id, or undefined for one this repository does not hold. */
function commitObject(repoRoot: string, commit: string): string | undefined {
  if (commit.length === 0 || commit.startsWith("-")) return undefined;
  const result = runGit(repoRoot, ["rev-parse", "--verify", "--quiet", "--end-of-options", `${commit}^{commit}`]);
  return result.status === 0 ? result.stdout.trim() || undefined : undefined;
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
  const entries = treeEntries(repoRoot, object);
  if (entries === undefined) return { candidates: [], problems: [`${CANDIDATE_DIRECTORY}: Git could not list it at ${commit}`], truncated: false };
  const named = new Map(entries.filter(entry => isCandidateFileName(entry.name)).map(entry => [entry.name, entry]));
  const { read, problems: extra, truncated } = capCandidateNames([...named.keys()]);
  const unreadable = (entry: TreeEntry): string | undefined => {
    if (entry.type !== "blob" || entry.mode !== "100644") return CANDIDATE_NOT_REGULAR;
    if (entry.size === null || entry.size > MAX_CANDIDATE_BYTES) return CANDIDATE_TOO_LARGE;
    return undefined;
  };
  const objects = read.map(name => named.get(name)!).filter(entry => unreadable(entry) === undefined).map(entry => entry.object);
  const blobs = await readGitBlobs(repoRoot, objects, { maxBlobBytes: MAX_CANDIDATE_BYTES });
  const candidates: Candidate[] = [];
  const problems: string[] = [];
  for (const name of read) {
    const entry = named.get(name)!;
    const problem = unreadable(entry);
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
  const entries = new Map((treeEntries(root, head) ?? []).map(entry => [entry.name, entry]));
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
