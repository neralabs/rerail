/**
 * Files Rerail reads from and writes to `.rerail/` folders other people's branches carry (candidates, decision
 * entries): data other people wrote. Reading follows no symlink (not `.rerail`, not the folder, not a file) and checks
 * each size before reading; writing creates the folders as needed, refuses a symlink in their place, and puts a file
 * in place by rename from a new temporary file. A Git tree is listed per folder, regular-file blobs only.
 */
import { randomBytes } from "node:crypto";
import { constants } from "node:fs";
import { chmod, lstat, mkdir, open, rename, unlink } from "node:fs/promises";
import { join } from "node:path";
import { runGit } from "../git.js";

/** `O_NOFOLLOW` where the platform has it; Windows has no symlink-following open to refuse. */
export const NO_FOLLOW = constants.O_NOFOLLOW ?? 0;
/** The problem for a name that is a folder, symlink or other non-regular entry, after `<file name>: `. */
export const NOT_REGULAR = "not a regular file";

export function errorCode(error: unknown): string | undefined {
  return typeof error === "object" && error !== null && "code" in error ? String(error.code) : undefined;
}

/** Names in byte order, so every machine reads the same first ones. */
export function byteOrder(left: string, right: string): number {
  return Buffer.compare(Buffer.from(left), Buffer.from(right));
}

/** The problem for a file over `maxBytes`, after `<file name>: `. */
export function tooLarge(maxBytes: number): string {
  return `larger than ${maxBytes} bytes`;
}

/**
 * Whether the folder at `path` (relative name `label`) is a real directory: false with no problem when it is missing,
 * false with a problem when it is anything else, a symlink included.
 */
export async function realFolder(path: string, label: string, problems: string[]): Promise<boolean> {
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

/** A file's text, opened without following a symlink, its size checked on the open file before reading. */
export async function readGuardedFile(path: string, name: string, maxBytes: number): Promise<{ text: string } | { problem: string }> {
  let handle;
  try {
    handle = await open(path, constants.O_RDONLY | NO_FOLLOW);
  } catch (error) {
    return { problem: `${name}: ${errorCode(error) === "ELOOP" ? NOT_REGULAR : `cannot be read (${errorCode(error) ?? "error"})`}` };
  }
  try {
    const entry = await handle.stat();
    if (!entry.isFile()) return { problem: `${name}: ${NOT_REGULAR}` };
    if (entry.size > maxBytes) return { problem: `${name}: ${tooLarge(maxBytes)}` };
    // One byte more than the limit, so a file that grew since the check is still caught.
    const buffer = Buffer.alloc(maxBytes + 1);
    let length = 0;
    for (;;) {
      const { bytesRead } = await handle.read(buffer, length, buffer.length - length, null);
      if (bytesRead === 0) break;
      length += bytesRead;
      if (length > maxBytes) return { problem: `${name}: ${tooLarge(maxBytes)}` };
    }
    return { text: buffer.subarray(0, length).toString("utf8") };
  } finally {
    await handle.close();
  }
}

/** Create the folder at `path` (0o755) unless it is there; refuse anything there that is not a real directory. */
async function ensureFolder(path: string, label: string): Promise<void> {
  try {
    await mkdir(path, { mode: 0o755 });
  } catch (error) {
    if (errorCode(error) !== "EEXIST") throw error;
  }
  const entry = await lstat(path);
  if (!entry.isDirectory()) throw new Error(`${label} is not a folder${entry.isSymbolicLink() ? " (a symbolic link, which Rerail does not follow)" : ""}, so nothing was written`);
}

/**
 * Write `text` to `root`'s `<directory>/<name>` (a directory one level under `.rerail`), mode 0o644 so it can be
 * committed: the folders are created as needed and refused when either is a symlink, the text goes to a new temporary
 * file opened exclusively without following a symlink, and a rename puts it in place. Returns the file's absolute path.
 */
export async function writeGuardedFile(root: string, directory: string, name: string, text: string): Promise<string> {
  const parent = directory.slice(0, directory.indexOf("/"));
  await ensureFolder(join(root, parent), parent);
  const folder = join(root, directory);
  await ensureFolder(folder, directory);
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

/** One `git ls-tree -l -z` entry directly in a folder. */
export interface TreeEntry {
  readonly mode: string;
  readonly type: string;
  readonly object: string;
  readonly size: number | null;
  readonly name: string;
}

/** `git ls-tree -l -z` records as entries named relative to `prefix`, those directly in it only. */
function parseTree(output: string, prefix: string): TreeEntry[] {
  return output.split("\0").flatMap(record => {
    // `<mode> SP <type> SP <object> SP+ <size> TAB <path>`; the size is `-` for a tree.
    const match = /^(\d{6}) (\w+) ([0-9a-f]+) +(\d+|-)\t(.+)$/su.exec(record);
    if (match === null) return [];
    const [, mode = "", type = "", object = "", size = "", path = ""] = match;
    if (!path.startsWith(prefix) || path.slice(prefix.length).includes("/")) return [];
    return [{ mode, type, object, size: size === "-" ? null : Number(size), name: path.slice(prefix.length) }];
  });
}

/** The entries of `git ls-tree -l -z <commit> -- <directory>/` directly in that folder, or undefined when Git fails. */
export function treeFolderEntries(repoRoot: string, commit: string, directory: string): TreeEntry[] | undefined {
  const result = runGit(repoRoot, ["ls-tree", "-l", "-z", "--full-tree", commit, "--", `${directory}/`], { maxBuffer: 16 * 1024 * 1024 });
  if (result.status !== 0 || result.error !== undefined) return undefined;
  return parseTree(result.stdout, `${directory}/`);
}

/** The entries of one tree object (`git ls-tree -l -z <tree>`), or undefined when Git fails. */
export function treeObjectEntries(repoRoot: string, tree: string): TreeEntry[] | undefined {
  if (!/^[0-9a-f]{40,64}$/u.test(tree)) return undefined;
  const result = runGit(repoRoot, ["ls-tree", "-l", "-z", tree], { maxBuffer: 16 * 1024 * 1024 });
  if (result.status !== 0 || result.error !== undefined) return undefined;
  return parseTree(result.stdout, "");
}

/** Why a listed entry is not read: not a regular-file blob, or over `maxBytes`; undefined when it is readable. */
export function unreadableEntry(entry: TreeEntry, maxBytes: number): string | undefined {
  if (entry.type !== "blob" || entry.mode !== "100644") return NOT_REGULAR;
  if (entry.size === null || entry.size > maxBytes) return tooLarge(maxBytes);
  return undefined;
}

/** A commit's object id, or undefined for one this repository does not hold. */
export function commitObject(repoRoot: string, commit: string): string | undefined {
  if (commit.length === 0 || commit.startsWith("-")) return undefined;
  const result = runGit(repoRoot, ["rev-parse", "--verify", "--quiet", "--end-of-options", `${commit}^{commit}`]);
  return result.status === 0 ? result.stdout.trim() || undefined : undefined;
}
