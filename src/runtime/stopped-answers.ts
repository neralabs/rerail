/**
 * The daemon's last answer of stopped for each folder a one-shot hook asked about, kept so the hooks after it in that
 * folder can repeat it while the daemon is slow to answer (`hookRerailStarted`).
 *
 * The daemon places a folder in a checkout once, with Git, and keeps that placement for its life
 * (`createCheckoutResolver`); each answer after that reads the database again. A hook cannot place a folder as Git does
 * from the disk alone, so it never decides stopped by itself: it repeats the daemon's own answer, and only while each
 * thing it can check still agrees with what was kept. Each check can only turn a kept stopped into not known, and the
 * hook then waits for the daemon as it always did.
 *
 * One file a folder, `stopped-answers/<daemon pid>-<daemon start>/<digest of the folder's real path>.json` in the Rerail
 * home. It holds no path: the repository key the database records, how many folders up the checkout's top is, the
 * device and inode of the folder, the top and the top's `.git`, and when the daemon answered. A new daemon places every
 * folder again, so the first hook that keeps an answer from it removes the folders of the daemons before it. A link in
 * place of the file or of either folder above it is never followed.
 */
import { createHash } from "node:crypto";
import {
  closeSync, constants, existsSync, lstatSync, mkdirSync, readdirSync, readFileSync, readlinkSync, realpathSync, rmSync, statSync, type BigIntStats,
} from "node:fs";
import { rm } from "node:fs/promises";
import { basename, dirname, join } from "node:path";
import { z } from "zod";
import { storedPath } from "../engine/catalog/checkout-control.js";
import type { RepositoryStartState } from "../engine/catalog/repository-model.js";
import { recordedCheckoutStopped } from "../engine/catalog/repository-registry.js";
import { replaceFile } from "../engine/file-replace.js";
import { openNoFollowSync } from "../engine/no-follow.js";
import { openRerailDatabaseAsIs, type RerailDatabase } from "../engine/storage/database.js";
import { DAEMON_STATE_FILE, defaultRerailHome } from "../engine/storage/rerail-home.js";

/** In the Rerail home: the daemon's last answer of stopped for each folder a hook asked about. */
export const STOPPED_ANSWERS_DIR = "stopped-answers";

/**
 * How long a kept answer may be repeated. Every repeat checks all the hook can check, so this bounds only what it cannot
 * see (`docs/reference.md` names it) to 30 minutes. It is long because only a daemon answer in time renews it, and a
 * daemon overloaded with other checkouts stayed so for over two hours on a laptop with 17 repositories: a shorter bound
 * would send the hooks back to waiting for it within minutes.
 */
const KEPT_FOR_MS = 30 * 60_000;

/** A hook whose daemon answered in time writes the answer again only once the kept one is this old, so most such hooks write nothing. */
const REWRITE_AFTER_MS = 60_000;

/** The daemon's answer to a hook's start check, with what it rests on (`readStartAnswer`). */
export interface StartAnswer {
  readonly state: RepositoryStartState;
  /** The folder as the daemon was asked about it: its real path. */
  readonly workspace: string;
  /** The checkout the daemon placed the folder in; null where it placed it in none it knows. */
  readonly scope: { readonly repositoryKey: string; readonly checkoutPath: string | null } | null;
  /** The daemon that answered, as `daemon.json` named it. */
  readonly daemon: DaemonIdentity;
}

interface DaemonIdentity {
  readonly pid: number;
  readonly startedAt: number;
}

const keptSchema = z.object({
  v: z.literal(1),
  repository: z.string(),
  depth: z.number().int().nonnegative(),
  folder: z.string(),
  checkout: z.string(),
  git: z.string(),
  at: z.number(),
});

/** What is kept of one answer of stopped. `folder`, `checkout` and `git` are `<device>:<inode>`. */
type KeptAnswer = z.infer<typeof keptSchema>;

/** Where the checkout's top is from the folder, and the device and inode of the folder, the top and the top's own `.git`. */
type Placement = Pick<KeptAnswer, "depth" | "folder" | "checkout" | "git">;

const daemonSchema = z.object({ pid: z.number().int(), startedAt: z.number(), databasePath: z.string() });

/** The daemon `daemon.json` names now, and the database it reads. */
type RecordedDaemon = z.infer<typeof daemonSchema>;

/**
 * Keeps `answer` for the hooks after this one in its folder: an answer of stopped that rests on a Git checkout the
 * daemon names, and that this folder is in as spelled, is written whole, replacing the one before; any other answer
 * removes the kept one. A failure costs only a later hook's wait.
 */
export async function keepStartAnswer(answer: StartAnswer, now = Date.now()): Promise<void> {
  const file = keptFile(answer.daemon, answer.workspace);
  if (onServer(answer.workspace) || !inFoldersOfItsOwn(file)) return;
  const kept = answer.state === "stopped" ? keptOf(answer, now) : undefined;
  await (kept === undefined ? rm(file, { force: true }) : rewritten(file, kept, now)).catch(() => undefined);
}

/**
 * Whether a hook in `workspace`, beside a daemon that has not answered yet, may answer stopped by repeating the daemon's
 * last answer there ({@link keepStartAnswer}): only while every check of {@link stillHolds} agrees with what was kept.
 * Throws only SQLite's error for a database it opened and could not read, which its caller reads as not known, and a
 * fault of its own.
 */
export function repeatsStoppedAnswer(workspace: string, now = Date.now()): boolean {
  const daemon = recordedDaemon();
  const folder = daemon === undefined ? undefined : localRealPath(workspace);
  if (daemon === undefined || folder === undefined) return false;
  const file = keptFile(daemon, folder);
  const kept = inFoldersOfItsOwn(file) ? readKept(file) : undefined;
  return kept !== undefined && stillHolds(kept, folder, daemon, now);
}

/**
 * Whether `kept` still holds for `folder`: it is recent; the folder, the checkout's top and the top's `.git` are the
 * same things on disk, with no `.git` between the folder and the top; the daemon holds no file of its database that is
 * gone from the disk; and the database, read whole and at once, still reads that checkout stopped.
 */
function stillHolds(kept: KeptAnswer, folder: string, daemon: RecordedDaemon, now: number): boolean {
  const top = folderUp(folder, kept.depth);
  return isRecent(kept, now) && sameOnDisk(kept, folder, top) && !daemonFileGone(daemon) && stillStopped(daemon.databasePath, top, kept.repository);
}

/** Kept less than `KEPT_FOR_MS` ago; one kept "later" than now, as after the clock was set back, is not. */
function isRecent(kept: KeptAnswer, now: number): boolean {
  return now >= kept.at && now - kept.at < KEPT_FOR_MS;
}

function sameOnDisk(kept: KeptAnswer, folder: string, top: string): boolean {
  const found = placementOf(folder, top);
  return found !== undefined && sameAnswer(found, kept) && noGitBetween(folder, kept.depth);
}

/**
 * What is kept of the stopped `answer`: undefined where the daemon placed the folder in no checkout, where the folder is
 * not that checkout's top or a folder in it as spelled, and where the top has no `.git` of its own. The daemon places a
 * folder outside Git again on every answer, so an answer resting on one is not kept.
 */
function keptOf({ workspace, scope }: StartAnswer, now: number): KeptAnswer | undefined {
  if (scope === null || scope.checkoutPath === null) return undefined;
  const placement = placementOf(workspace, scope.checkoutPath);
  return placement === undefined ? undefined : { v: 1, repository: scope.repositoryKey, ...placement, at: now };
}

/** Where `top` is from `folder` and what each is on disk; undefined where `top` is not `folder` or above it, or one cannot be read. */
function placementOf(folder: string, top: string): Placement | undefined {
  const depth = depthOf(folder, top);
  const [own, checkout, git] = [idAt(folder, statSync), idAt(top, statSync), idAt(join(top, ".git"), lstatSync)];
  return depth === undefined || own === undefined || checkout === undefined || git === undefined ? undefined : { depth, folder: own, checkout, git };
}

/** Writes `kept` at `file`, unless what is there says the same and is younger than `REWRITE_AFTER_MS`. */
async function rewritten(file: string, kept: KeptAnswer, now: number): Promise<void> {
  if (keptAlready(readKept(file), kept, now)) return;
  madeForThisDaemon(dirname(file));
  await replaceFile(file, JSON.stringify(kept), { mode: 0o600, retryMs: 0 });
}

function keptAlready(before: KeptAnswer | undefined, kept: KeptAnswer, now: number): boolean {
  return before !== undefined && sameAnswer(before, kept) && now >= before.at && now - before.at < REWRITE_AFTER_MS;
}

/** Whether `left` and `right` say the same of the folder's placement: the repository where both name one, and the depth and the disk ids. */
function sameAnswer(left: Placement & { repository?: string }, right: KeptAnswer): boolean {
  return (left.repository === undefined || left.repository === right.repository) && left.depth === right.depth
    && left.folder === right.folder && left.checkout === right.checkout && left.git === right.git;
}

/** Makes `folder`, this daemon's; the hook that makes it removes the folders of the daemons before it, whose answers are never repeated. */
function madeForThisDaemon(folder: string): void {
  if (mkdirSync(folder, { recursive: true, mode: 0o700 }) === undefined) return;
  const earlier = readdirSync(dirname(folder)).filter(name => name !== basename(folder));
  for (const name of earlier) rmSync(join(dirname(folder), name), { recursive: true, force: true });
}

/**
 * Whether the two folders that hold the kept answer `file` in the Rerail home are each a folder of its own or not there
 * yet: Rerail makes them, so a link in their place, which can reach a server that does not answer, is never followed.
 */
function inFoldersOfItsOwn(file: string): boolean {
  return [dirname(dirname(file)), dirname(file)].every(folder => entryKind(folder) !== "other");
}

/** What is at `path`, not followed through a link: nothing, a folder, or anything else, a look that fails included. */
function entryKind(path: string): "none" | "folder" | "other" {
  try {
    const stats = lstatSync(path, { throwIfNoEntry: false });
    return stats === undefined ? "none" : stats.isDirectory() ? "folder" : "other";
  } catch {
    return "other";
  }
}

/** The file of the answer `daemon` gave for `folder`, a real path. */
function keptFile(daemon: DaemonIdentity, folder: string, home = defaultRerailHome()): string {
  const name = createHash("sha256").update(folder).digest("hex").slice(0, 32);
  return join(home, STOPPED_ANSWERS_DIR, `${daemon.pid}-${daemon.startedAt}`, `${name}.json`);
}

/** The answer kept at `file`; undefined where none is there whole, a look fails, or a link is at its name, which is never followed. */
function readKept(file: string): KeptAnswer | undefined {
  try {
    const descriptor = openNoFollowSync(file, constants.O_RDONLY);
    try {
      return keptSchema.safeParse(JSON.parse(readFileSync(descriptor, "utf8"))).data;
    } finally {
      closeSync(descriptor);
    }
  } catch {
    return undefined;
  }
}

function recordedDaemon(home = defaultRerailHome()): RecordedDaemon | undefined {
  try {
    return daemonSchema.safeParse(JSON.parse(readFileSync(join(home, DAEMON_STATE_FILE), "utf8"))).data;
  } catch {
    return undefined;
  }
}

/** How many folders up from `folder` the folder `top` is, as both are spelled; undefined where it is not above it. */
function depthOf(folder: string, top: string): number | undefined {
  let depth = 0;
  for (let at = folder; at !== top; at = dirname(at), depth += 1) {
    if (dirname(at) === at) return undefined;
  }
  return depth;
}

/** The folder `depth` folders up from `folder`. */
function folderUp(folder: string, depth: number): string {
  let at = folder;
  for (let up = 0; up < depth; up += 1) at = dirname(at);
  return at;
}

/**
 * Whether no `.git` is in `folder` nor in a folder above it short of `depth` up, where the kept top is: one there now,
 * or one a look cannot settle, means Git may place the folder in another checkout.
 */
function noGitBetween(folder: string, depth: number): boolean {
  let at = folder;
  for (let up = 0; up < depth; up += 1, at = dirname(at)) {
    if (entryKind(join(at, ".git")) !== "none") return false;
  }
  return true;
}

/** `<device>:<inode>` of `path` as `look` gives it; undefined where it fails or the filesystem gives no inode. */
function idAt(path: string, look: (path: string, options: { bigint: true }) => BigIntStats): string | undefined {
  try {
    const { dev, ino } = look(path, { bigint: true });
    return ino === 0n ? undefined : `${dev}:${ino}`;
  } catch {
    return undefined;
  }
}

/**
 * `workspace`'s real path as the hook asks the daemon about it (the native realpath, `readStartAnswer`); undefined where
 * it has none, and where it or its real path is on a server, which is never touched.
 */
function localRealPath(workspace: string): string | undefined {
  const real = onServer(workspace) ? undefined : realPathOf(workspace);
  return real === undefined || onServer(real) ? undefined : real;
}

function realPathOf(workspace: string): string | undefined {
  try {
    return existsSync(workspace) ? storedPath(workspace) : undefined;
  } catch {
    return undefined;
  }
}

/** A Windows path on a server (`\\server\share`, `\\?\UNC\server\share`): where the server does not answer, Windows holds each look there about 21 s. */
const SERVER_PATH = /^[\\/]{2}(?:[?.][\\/]UNC[\\/])?[^\\/?.]/i;

function onServer(path: string): boolean {
  return process.platform === "win32" && SERVER_PATH.test(path);
}

/**
 * On Linux, whether the daemon holds open a file of its database that is gone from the disk: one deleted or replaced
 * under it, as removing the WAL by hand or restoring a backup over the file does. Another reader then makes new files
 * and reads the database without what the daemon wrote since its last checkpoint. A daemon whose open files cannot be
 * listed counts as one. Elsewhere false: Windows refuses to delete a file SQLite holds, and macOS has no `/proc`.
 */
function daemonFileGone({ pid, databasePath }: RecordedDaemon): boolean {
  if (process.platform !== "linux") return false;
  try {
    const deleted = deletedNames(databasePath);
    return readdirSync(`/proc/${pid}/fd`).some(fd => deleted.has(linkTarget(`/proc/${pid}/fd/${fd}`)));
  } catch {
    return true;
  }
}

/** What `/proc/<pid>/fd` names a file of the database at `databasePath` once it is deleted: its real path and ` (deleted)`. */
function deletedNames(databasePath: string): Set<string> {
  const file = join(realpathSync(dirname(databasePath)), basename(databasePath));
  return new Set(["", "-wal", "-shm"].map(suffix => `${file}${suffix} (deleted)`));
}

/** Where the link at `path` points; empty where it is gone, as a descriptor closed while the list was read is. */
function linkTarget(path: string): string {
  try {
    return readlinkSync(path);
  } catch {
    return "";
  }
}

/**
 * Whether the database at `databasePath`, opened as it is beside the daemon (`besideDaemon`: never written, never
 * waiting on a lock, refused where its WAL or the WAL's index is gone), still reads the checkout recorded at `top` as a
 * stopped checkout of `repository` (`recordedCheckoutStopped`). False where it cannot be opened so; SQLite's error for
 * one it opened and could not read is thrown.
 */
function stillStopped(databasePath: string, top: string, repository: string): boolean {
  let database: RerailDatabase;
  try {
    database = openRerailDatabaseAsIs(databasePath, { besideDaemon: true });
  } catch {
    return false;
  }
  try {
    return recordedCheckoutStopped(database, top, repository);
  } finally {
    database.$client.close();
  }
}
