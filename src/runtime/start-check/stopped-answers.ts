/**
 * The daemon's last answer of stopped for each folder a one-shot hook asked about, kept so the hooks after it in that
 * folder can repeat it while the daemon is slow to answer (`hookRerailStarted`).
 *
 * The daemon places a folder in a checkout once, with Git, and keeps that placement for its life
 * (`createCheckoutResolver`); each answer after that reads the checkout's clone id file and the database again
 * (`lookupAt`). A hook cannot place a folder as Git does from the disk alone, so it never decides stopped by itself: it
 * repeats the daemon's own answer, and only while each thing it can check still agrees with what was kept, the clone id
 * the daemon reads and its rule for it among them. Each check can only turn a kept stopped into not known, and the hook
 * then waits for the daemon as it always did.
 *
 * One file a folder, `stopped-answers/<daemon pid>-<daemon start>/<digest of the folder's real path>.json` in the Rerail
 * home. It holds no path: the repository key and the clone id the database records, how many folders up the checkout's
 * top is, the device and inode of the folder, the top and the top's `.git`, those of the database the daemon holds, its
 * WAL and the WAL's index, the start mark the hook found before it asked the daemon (`start-mark.ts`), and when the
 * daemon answered. A new daemon
 * places every folder again, so it removes the folders of the daemons before it once it runs (`removeEarlierAnswers`),
 * and no hook does. A link in place of the file or of either folder above it is never followed. The hook reads the file,
 * and the checkout's `.git`, `commondir` and clone id files, only as small plain files opened without waiting, so a pipe
 * at one's name never holds it.
 */
import { createHash } from "node:crypto";
import {
  closeSync, constants, existsSync, fstatSync, lstatSync, mkdirSync, readdirSync, readFileSync, readSync, statSync, type BigIntStats,
} from "node:fs";
import { readdir, rm } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { z } from "zod";
import { storedPath } from "../../engine/catalog/checkout-control.js";
import type { RepositoryStartState } from "../../engine/catalog/repository-model.js";
import { readCloneIdAt } from "../../engine/catalog/repository-identity.js";
import { recordedCheckoutStopped } from "../../engine/catalog/repository-registry.js";
import { START_MARK, STOPPED_ANSWERS_DIR, STUCK_MARK, startMarkFound, startMarkHolds } from "../../engine/storage/start-mark.js";
import { replaceFile } from "../../engine/file-replace.js";
import { openNoFollowSync } from "../../engine/no-follow.js";
import { defaultRerailDatabasePath, openRerailDatabaseAsIs, type RerailDatabase } from "../../engine/storage/database.js";
import { DAEMON_STATE_FILE, defaultRerailHome } from "../../engine/storage/rerail-home.js";

export { STOPPED_ANSWERS_DIR } from "../../engine/storage/start-mark.js";

/**
 * How long a kept answer may be repeated. Every repeat checks all the hook can check, so this bounds only what it cannot
 * see (`docs/reference.md` names it) to 30 minutes. It is long because only a daemon answer in time renews it, and a
 * daemon overloaded with other checkouts stayed so for over two hours on a laptop with 17 repositories: a shorter bound
 * would send the hooks back to waiting for it within minutes.
 */
const KEPT_FOR_MS = 30 * 60_000;

/** A hook whose daemon answered in time writes the answer again only once the kept one is this old, so most such hooks write nothing. */
const REWRITE_AFTER_MS = 60_000;

/** The clone id file in Git's common folder, and the one a folder the daemon places outside Git keeps (`repositoryIdentityLocation`). */
const CLONE_ID_FILE = "rerail-repository-id";
const OUTSIDE_GIT_ID_FOLDER = ".rerail";
const OUTSIDE_GIT_ID_FILE = "repository-id";

/** The most a `.git` file or a `commondir` file may hold here: Git writes one short line in each. */
const MAX_POINTER_BYTES = 4_096;

/** The most a kept answer may hold: one takes about 250 bytes, and only a repository key thousands of characters long comes near. */
const MAX_KEPT_BYTES = 16_384;

/** Opens a file without waiting for a writer where the OS can, so a pipe planted at the name of a file the hook reads cannot hold it. */
const AT_ONCE = constants.O_NONBLOCK ?? 0;

/** The daemon's answer to a hook's start check, with what it rests on (`readStartAnswer`). */
export interface StartAnswer {
  readonly state: RepositoryStartState;
  /** The folder as the daemon was asked about it: its real path. */
  readonly workspace: string;
  /** The checkout the daemon placed the folder in; null where it placed it in none it knows. */
  readonly scope: { readonly repositoryKey: string; readonly checkoutPath: string | null } | null;
  /** The daemon that answered, as `daemon.json` named it, and the database it reads. */
  readonly daemon: DaemonIdentity & { readonly databasePath: string };
}

/** A daemon as `daemon.json` names it: its process and when it started. */
export interface DaemonIdentity {
  readonly pid: number;
  readonly startedAt: number;
}

const keptSchema = z.object({
  v: z.literal(4),
  repository: z.string(),
  cloneId: z.string(),
  depth: z.number().int().nonnegative(),
  folder: z.string(),
  checkout: z.string(),
  git: z.string(),
  database: z.string(),
  wal: z.string(),
  walIndex: z.string(),
  mark: z.string(),
  at: z.number(),
});

/**
 * What is kept of one answer of stopped. `cloneId` is the clone id the checkout's Git folder held when it was kept;
 * `folder`, `checkout`, `git`, `database`, `wal` and `walIndex` are `<device>:<inode>`; `mark` is the start mark the
 * hook found before it asked the daemon (`startMarkFound`).
 */
type KeptAnswer = z.infer<typeof keptSchema>;

/** The start mark a hook found before it asked the daemon (`startMarkFound`), beside the database at `databasePath`. */
export interface MarkBeforeAsking {
  readonly databasePath: string;
  readonly mark: string | undefined;
}

/** Where the checkout's top is from the folder, and the device and inode of the folder, the top and the top's own `.git`. */
type Placement = Pick<KeptAnswer, "depth" | "folder" | "checkout" | "git">;

/** The device and inode of the database file, its WAL (`-wal`) and the WAL's index (`-shm`). */
type DatabaseFiles = Pick<KeptAnswer, "database" | "wal" | "walIndex">;

/**
 * What is kept of an answer of stopped but its clone id, which is read only when the answer is written; the checkout's
 * top; and the process of the daemon that answered.
 */
interface Placed {
  readonly kept: Omit<KeptAnswer, "cloneId">;
  readonly top: string;
  readonly pid: number;
}

const daemonSchema = z.object({ pid: z.number().int(), startedAt: z.number(), databasePath: z.string() });

/** The daemon `daemon.json` names now, and the database it reads. */
type RecordedDaemon = z.infer<typeof daemonSchema>;

/**
 * The start mark beside the Rerail database, as a hook finds it before it asks the daemon: a change to where Rerail runs
 * that commits after this renews it, so an answer kept with it is repeated only while no such change has committed since
 * ({@link keepStartAnswer}).
 */
export function markBeforeAsking(): MarkBeforeAsking {
  const databasePath = defaultRerailDatabasePath();
  return { databasePath, mark: startMarkFound(databasePath) };
}

/**
 * Keeps `answer` for the hooks after this one in its folder: an answer of stopped that rests on a Git checkout the
 * daemon names, and that this folder is in as spelled, is written whole with the start mark found `before` the daemon
 * was asked, replacing the one before; any other answer removes the kept one. A failure costs only a later hook's wait.
 */
export async function keepStartAnswer(answer: StartAnswer, before: MarkBeforeAsking, now = Date.now()): Promise<void> {
  const file = keptFile(answer.daemon, answer.workspace);
  if (onServer(answer.workspace) || !inFoldersOfItsOwn(file)) return;
  const placed = answer.state === "stopped" ? placedOf(answer, markBeside(answer.daemon.databasePath, before), now) : undefined;
  await (placed === undefined ? rm(file, { force: true }) : rewritten(file, placed, now)).catch(() => undefined);
}

/** The mark found `before` the daemon was asked, where it lies beside the database at `databasePath`, the one the daemon reads. */
function markBeside(databasePath: string, before: MarkBeforeAsking): string | undefined {
  return resolve(databasePath) === resolve(before.databasePath) ? before.mark : undefined;
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
 * Whether `kept` still holds for `folder`: it is recent; the start mark is the one found before the daemon gave it, so
 * no change to where Rerail runs has committed since, which the daemon may have read though the database file was later
 * put back as it was ({@link startMarkHolds}); the folder, the checkout's top and the top's `.git` have the same device
 * and file ids, with no `.git` between the folder and the top; the top holds the clone id that was kept, where the
 * daemon reads it; the database, its WAL and the WAL's index at their names are the files the daemon held when it
 * answered ({@link sameDatabaseFiles}); and the database, read whole and at once, still places the checkout by that
 * clone id in the kept repository and reads it stopped.
 */
function stillHolds(kept: KeptAnswer, folder: string, daemon: RecordedDaemon, now: number): boolean {
  const top = folderUp(folder, kept.depth);
  return within(kept, now, KEPT_FOR_MS) && startMarkHolds(daemon.databasePath, kept.mark) && sameOnDisk(kept, folder, top)
    && sameCloneId(top, kept.cloneId) && sameDatabaseFiles(kept, daemon.databasePath) && stillStopped(daemon.databasePath, top, kept);
}

/** Kept less than `ms` ago; one kept "later" than now, as after the clock was set back, is not. */
function within(kept: Pick<KeptAnswer, "at">, now: number, ms: number): boolean {
  return now >= kept.at && now - kept.at < ms;
}

function sameOnDisk(kept: KeptAnswer, folder: string, top: string): boolean {
  const found = placementOf(folder, top);
  return found !== undefined && sameAnswer(found, kept) && noGitBetween(folder, kept.depth);
}

/**
 * What is kept of the stopped `answer` with the start `mark`, but its clone id: undefined where no mark was found beside
 * the database the daemon reads, where the daemon placed the folder in no checkout, where the folder is not that
 * checkout's top or a folder in it as spelled, where the top has no `.git` of its own, and where a look at the database
 * the daemon reads, its WAL or the WAL's index fails. The daemon places a folder outside Git again on every answer, so
 * an answer resting on one is not kept.
 */
function placedOf({ workspace, scope, daemon }: StartAnswer, mark: string | undefined, now: number): Placed | undefined {
  if (mark === undefined || scope === null || scope.checkoutPath === null) return undefined;
  const placement = placementOf(workspace, scope.checkoutPath);
  const files = databaseFilesAt(daemon.databasePath);
  if (placement === undefined || files === undefined) return undefined;
  return { kept: { v: 4, repository: scope.repositoryKey, ...placement, ...files, mark, at: now }, top: scope.checkoutPath, pid: daemon.pid };
}

/** Where `top` is from `folder` and what each is on disk; undefined where `top` is not `folder` or above it, or one cannot be read. */
function placementOf(folder: string, top: string): Placement | undefined {
  const depth = depthOf(folder, top);
  const [own, checkout, git] = [idAt(folder, statSync), idAt(top, statSync), idAt(join(top, ".git"), lstatSync)];
  return depth === undefined || own === undefined || checkout === undefined || git === undefined ? undefined : { depth, folder: own, checkout, git };
}

/**
 * Writes the answer `placed` names at `file`, with the clone id the checkout's top holds now, unless what is there says
 * the same of the folder's placement and the start mark and is younger than `REWRITE_AFTER_MS`, the top holds no clone
 * id this hook can read, which no later hook could check, or the daemon does not hold open the files at the database's
 * names ({@link daemonHolds}). It makes the daemon's own folder only inside the answers folder a daemon made, never
 * that one: a mark found in a folder since removed is one no renewal reached, and an answer kept with it is dropped.
 */
async function rewritten(file: string, { kept, top, pid }: Placed, now: number): Promise<void> {
  if (keptAlready(readKept(file), kept, now)) return;
  const cloneId = cloneIdAt(top);
  if (cloneId === undefined || !daemonHolds(pid, kept)) return;
  folderMadeIn(dirname(file));
  await replaceFile(file, JSON.stringify({ ...kept, cloneId }), { mode: 0o600, retryMs: 0 });
}

/** Makes `folder`, 0700, where it is not there; throws where its parent is not there, or where any other step fails. */
function folderMadeIn(folder: string): void {
  try {
    mkdirSync(folder, { mode: 0o700 });
  } catch (error) {
    if (!(error instanceof Error && "code" in error && error.code === "EEXIST")) throw error;
  }
}

function keptAlready(before: KeptAnswer | undefined, kept: Placed["kept"], now: number): boolean {
  return before !== undefined && before.mark === kept.mark && sameAnswer(before, kept) && within(before, now, REWRITE_AFTER_MS);
}

/** Whether `left` and `right` say the same of the folder's placement: the repository where both name one, and the depth and the disk ids. */
function sameAnswer(left: Placement & { repository?: string }, right: Placement & { repository: string }): boolean {
  return (left.repository === undefined || left.repository === right.repository) && left.depth === right.depth
    && left.folder === right.folder && left.checkout === right.checkout && left.git === right.git;
}

/**
 * Makes the folder of kept answers in the Rerail home `home` where nothing is at its name, 0700. Only a daemon makes it,
 * as it starts: from then on each change to where Rerail runs renews the start mark in it, and hooks keep this daemon's
 * answers there. A home no daemon ran in has no such folder, so a start or a stop there writes nothing beside the
 * database. Anything at the name is left as it is: a link, which is never followed, a file, or the folder itself. Where
 * the home takes no new folder, no hook keeps an answer and each waits for the daemon.
 */
export function makeAnswersFolder(home: string): void {
  try {
    mkdirSync(join(home, STOPPED_ANSWERS_DIR), { mode: 0o700 });
  } catch {
    // Something is at the name already, or the home takes no new folder.
  }
}

/** The files beside the daemons' folders of kept answers: the start mark and the stuck mark (`start-mark.ts`). */
const MARKS: ReadonlySet<string> = new Set([START_MARK, STUCK_MARK]);

/**
 * Removes from the Rerail home `home` the folders of answers kept from every daemon but `daemon`, whose answers are never
 * repeated: the daemon calls it once it has written `daemon.json`, off its start, so no hook pays for it. Each entry is
 * removed on its own, a link as a link and never what it leads to; one refused now (Windows holding a file in it a
 * moment) is left for the next daemon. The start mark and the stuck mark stay: a writer may be renewing them now.
 */
export async function removeEarlierAnswers(home: string, daemon: DaemonIdentity): Promise<void> {
  const answers = join(home, STOPPED_ANSWERS_DIR);
  if (entryKind(answers) !== "folder") return;
  for (const name of await readdir(answers).catch(() => [])) {
    if (name !== daemonFolder(daemon) && !MARKS.has(name)) await removeEntry(join(answers, name));
  }
}

/** Removes `path`, a folder with what is in it or anything else alone, never through a link; a removal refused is left. */
async function removeEntry(path: string): Promise<void> {
  await rm(path, { recursive: entryKind(path) === "folder", force: true }).catch(() => undefined);
}

/**
 * Whether the two folders that hold the kept answer `file` in the Rerail home are each a folder of its own or not there
 * yet: Rerail makes them, so a link in their place, which can reach a server that does not answer, is never followed.
 */
function inFoldersOfItsOwn(file: string): boolean {
  return [dirname(dirname(file)), dirname(file)].every(folder => FOLDER_OR_NOTHING.has(entryKind(folder)));
}

const FOLDER_OR_NOTHING: ReadonlySet<string> = new Set(["none", "folder"]);

/** What is at `path`, not followed through a link: nothing, a folder, a plain file, or anything else, a look that fails included. */
function entryKind(path: string): "none" | "folder" | "file" | "other" {
  try {
    const stats = lstatSync(path, { throwIfNoEntry: false });
    if (stats === undefined) return "none";
    return stats.isDirectory() ? "folder" : stats.isFile() ? "file" : "other";
  } catch {
    return "other";
  }
}

/**
 * Whether the checkout's top `top` holds the clone id `cloneId` where the daemon reads it on every answer
 * (`lookupAt`): in the clone id file of the Git common folder its `.git` names ({@link cloneIdAt}), and, for a folder
 * whose `.git` the daemon's Git does not take and which it then places outside Git, in `.rerail/repository-id`, which
 * may be missing or unreadable, as `lookupAt` reads it then as no id, or name the same clone.
 */
function sameCloneId(top: string, cloneId: string): boolean {
  return cloneIdAt(top) === cloneId && outsideGitIdAgrees(top, cloneId);
}

function outsideGitIdAgrees(top: string, cloneId: string): boolean {
  const folder = join(top, OUTSIDE_GIT_ID_FOLDER);
  const kind = entryKind(folder);
  if (kind !== "folder") return kind === "none";
  const id = readCloneIdAt(join(folder, OUTSIDE_GIT_ID_FILE), { atOnce: true });
  return id === undefined || id === cloneId;
}

/**
 * The clone id in the clone id file of the Git common folder that `top`'s own `.git` names, read as the daemon reads it
 * (`readCloneIdAt`) but never waiting on a pipe there, and found as Git finds it, without running Git: `.git` is that
 * Git folder, or a file whose `gitdir:` line names it, and the folder's `commondir` names the common folder where it has
 * one. Undefined where a look fails, a link or a path on a server is on the way, which is never followed, or the file
 * holds no clone id.
 */
function cloneIdAt(top: string): string | undefined {
  const common = commonFolderOf(gitFolderOf(top));
  return common === undefined ? undefined : readCloneIdAt(join(common, CLONE_ID_FILE), { atOnce: true });
}

/** `top`'s Git folder: its `.git` folder, or the folder its `.git` file names (Git's `read_gitfile`). */
function gitFolderOf(top: string): string | undefined {
  const dotGit = join(top, ".git");
  const kind = entryKind(dotGit);
  if (kind === "folder") return dotGit;
  const text = kind === "file" ? smallFileText(dotGit, MAX_POINTER_BYTES) : undefined;
  return typeof text === "string" && text.startsWith("gitdir: ") ? namedFolder(top, text.slice("gitdir: ".length)) : undefined;
}

/** The common folder of the Git folder `git` (Git's `get_common_dir`): the folder its `commondir` file names, else `git`. */
function commonFolderOf(git: string | undefined): string | undefined {
  if (git === undefined) return undefined;
  const text = smallFileText(join(git, "commondir"), MAX_POINTER_BYTES);
  if (text === null) return git;
  return text === undefined ? undefined : namedFolder(git, text);
}

/**
 * The folder the line `text` of a `.git` or `commondir` file names, its line end cut off and resolved against `base`, as
 * Git reads it; undefined where it names none, or where it or a folder above it is a link, is not a folder, or is on a
 * server. Each is looked at from the top down, so no look passes through a link, which can lead to a server that does
 * not answer.
 */
function namedFolder(base: string, text: string): string | undefined {
  const named = text.replace(/[\r\n]+$/u, "");
  const folder = resolve(base, named);
  return named !== "" && !onServer(folder) && foldersDownTo(folder).every(each => entryKind(each) === "folder") ? folder : undefined;
}

/** The folders from just below `path`'s root down to `path`, the topmost first. */
function foldersDownTo(path: string): string[] {
  const folders: string[] = [];
  for (let at = path; dirname(at) !== at; at = dirname(at)) folders.unshift(at);
  return folders;
}

/**
 * The text of the small file at `path` (a `.git` file, a `commondir`, a kept answer), read without following a link at
 * its name or waiting on a pipe or a device there: null where nothing is there, undefined where it cannot be opened,
 * is not a plain file, or holds more than `most` bytes.
 */
function smallFileText(path: string, most: number): string | null | undefined {
  let descriptor: number;
  try {
    descriptor = openNoFollowSync(path, constants.O_RDONLY | AT_ONCE);
  } catch (error) {
    return error instanceof Error && "code" in error && error.code === "ENOENT" ? null : undefined;
  }
  try {
    return plainFileText(descriptor, most);
  } finally {
    closeSync(descriptor);
  }
}

/** The text of the open file `descriptor`, where it is a plain file of at most `most` bytes; undefined otherwise. */
function plainFileText(descriptor: number, most: number): string | undefined {
  try {
    const buffer = Buffer.alloc(most + 1);
    const length = fstatSync(descriptor).isFile() ? readSync(descriptor, buffer, 0, buffer.length, 0) : buffer.length;
    return length > most ? undefined : buffer.subarray(0, length).toString("utf8");
  } catch {
    return undefined;
  }
}

/** The file of the answer `daemon` gave for `folder`, a real path. */
function keptFile(daemon: DaemonIdentity, folder: string, home = defaultRerailHome()): string {
  const name = createHash("sha256").update(folder).digest("hex").slice(0, 32);
  return join(home, STOPPED_ANSWERS_DIR, daemonFolder(daemon), `${name}.json`);
}

/** The name of the folder of the answers `daemon` gave. */
function daemonFolder({ pid, startedAt }: DaemonIdentity): string {
  return `${pid}-${startedAt}`;
}

/**
 * The answer kept at `file`; undefined where none is there whole, a look fails, or what is at its name is not a plain
 * file of at most `MAX_KEPT_BYTES`: a link is never followed, and a pipe or a device is never waited on.
 */
function readKept(file: string): KeptAnswer | undefined {
  const text = smallFileText(file, MAX_KEPT_BYTES);
  if (typeof text !== "string") return undefined;
  try {
    return keptSchema.safeParse(JSON.parse(text)).data;
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
 * The device and inode of the database at `databasePath`, its WAL and the WAL's index, each looked at without following
 * a link; undefined where a look fails. The daemon's connections hold all three open for its whole life, so SQLite
 * never deletes or makes again the WAL or its index under it, and a checkpoint or `VACUUM` writes each in place.
 */
function databaseFilesAt(databasePath: string): DatabaseFiles | undefined {
  const [database, wal, walIndex] = ["", "-wal", "-shm"].map(suffix => idAt(`${databasePath}${suffix}`, lstatSync));
  return database === undefined || wal === undefined || walIndex === undefined ? undefined : { database, wal, walIndex };
}

/**
 * Whether the database at `databasePath`, its WAL and the WAL's index are the files the answer `kept` was kept beside,
 * the ones the daemon held: one moved aside, deleted or replaced under the running daemon since, as a restore that skips
 * `rerail daemon stop` does, has another file id at its name, and another reader would read a file the daemon does not.
 * A file the daemon still holds keeps its id, so no other file can take it.
 */
function sameDatabaseFiles(kept: DatabaseFiles, databasePath: string): boolean {
  const now = databaseFilesAt(databasePath);
  return now !== undefined && sameFiles(now, kept);
}

function sameFiles(left: DatabaseFiles, right: DatabaseFiles): boolean {
  return left.database === right.database && left.wal === right.wal && left.walIndex === right.walIndex;
}

/**
 * On Linux, whether the daemon's process `pid` holds open each of `files`, by device and inode: one moved aside or
 * replaced under it before it answered is not at its name, and an answer kept beside it would be checked against a file
 * the daemon does not read. A daemon whose open files cannot be listed holds none. Elsewhere true: Windows refuses to
 * move or delete a file SQLite holds, and macOS has no `/proc`.
 */
function daemonHolds(pid: number, files: DatabaseFiles): boolean {
  if (process.platform !== "linux") return true;
  try {
    const held = new Set(readdirSync(`/proc/${pid}/fd`).map(fd => idAt(`/proc/${pid}/fd/${fd}`, statSync)));
    return [files.database, files.wal, files.walIndex].every(id => held.has(id));
  } catch {
    return false;
  }
}

/**
 * Whether the database at `databasePath`, opened as it is beside the daemon (`besideDaemon`: never written, never
 * waiting on a lock, refused where its WAL or the WAL's index is gone), still places the checkout recorded at `top` by
 * the kept clone id in the kept repository and reads it stopped (`recordedCheckoutStopped`, the daemon's own rule). False
 * where it cannot be opened so; SQLite's error for one it opened and could not read is thrown.
 */
function stillStopped(databasePath: string, top: string, { repository, cloneId }: KeptAnswer): boolean {
  let database: RerailDatabase;
  try {
    database = openRerailDatabaseAsIs(databasePath, { besideDaemon: true });
  } catch {
    return false;
  }
  try {
    return recordedCheckoutStopped(database, top, repository, cloneId);
  } finally {
    database.$client.close();
  }
}
