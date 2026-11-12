/**
 * The run folder: one folder per vitest run that holds every temporary folder the run's tests make, and those of the
 * processes they start, so one removal when the run ends takes them all, a folder a test never removed included.
 * vitest.config.ts loads this module as a global setup, in vitest's own process before the first test file: it points
 * TMPDIR, TEMP and TMP at the folder, which `os.tmpdir()` reads, and the workers and every process they start inherit.
 *
 * A run killed before it ends (a closed terminal, a tool call's time limit) leaves its folder behind, and a later run
 * removes it once that run has ended. The pid in a run folder's name tells that only where the pid means the same
 * process: a container or sandbox with a pid namespace of its own, or another host, may share the temp folder. So each
 * run folder holds an owner file that names where its run's pid means something (the host and, on Linux, the pid
 * namespace and the boot), and the run touches the file every minute while it runs. A run removes a run folder from
 * its own place once no process has the pid, one from another place once its owner file has gone ten minutes
 * untouched, and any run folder once its owner file has gone a day untouched, since the pid may by then belong to
 * another process. A removal that leaves part of a run folder puts its owner file back as it was. Any other
 * `rerail-vitest-*` folder goes once it is a day old: test runs made each test file's home and store
 * (`rerail-vitest-home-*`, `rerail-vitest-store-*`) straight in the temp folder before there was a run folder, and a
 * checkout on an older commit may still be running its tests in one. A run picks those folders at its start and
 * removes them while its tests run, so its first test file waits for none of them; its end waits for that removal.
 */
import { readFileSync, readlinkSync, type Stats } from "node:fs";
import { chmod, lstat, mkdtemp, readdir, readFile, rm, utimes, writeFile } from "node:fs/promises";
import { hostname, tmpdir } from "node:os";
import { basename, join, sep } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const PREFIX = "rerail-vitest-";
/** A run folder's name: the prefix, the pid of the vitest process that made it, then mkdtemp's six characters. */
const RUN_FOLDER = /^rerail-vitest-(\d+)-[A-Za-z0-9]{6}$/u;
/** The file in a run folder that names where its run's pid means something, and that the run touches while it runs. */
export const OWNER_FILE = ".owner";
const MINUTE_MS = 60 * 1000;
const DAY_MS = 24 * 60 * MINUTE_MS;
/** How often a run touches its owner file. */
const HEARTBEAT_MS = MINUTE_MS;
/** How long the owner file of a run from another place may go untouched before that run counts as ended. */
const QUIET_MS = 10 * MINUTE_MS;
const VARIABLES = ["TMPDIR", "TEMP", "TMP"] as const;
/** Where a run folder goes when the temp folder leaves no room below it for the sockets, unless the variable names another folder. */
const SHORT_TEMP_VARIABLE = "RERAIL_TEST_SHORT_TEMP";
const SHORT_TEMP = "/tmp";
/** The longest Unix socket path in bytes, the terminating NUL aside, as src/runtime/engine-service.ts counts it. */
const SOCKET_PATH_BYTES = process.platform === "linux" || process.platform === "android" ? 107 : 103;
/**
 * How far below its temp folder the deepest Unix socket a test binds ends, in bytes: engine-service-socket.test.ts's
 * `/rerail-socket-XXXXXX/rerail-socket/nested/engine.sock`. A daemon's default socket, `/rerail-<uid>/<24 hex>.sock`,
 * ends 42 bytes below it.
 */
const DEEPEST_SOCKET_BYTES = 54;
/** The errors Windows gives while a process still holds a file in a folder being removed. */
const HELD = new Set(["EBUSY", "EPERM", "ENOTEMPTY", "EACCES"]);
/** The errors a removal gives for want of permission, on Linux and macOS when a folder in the tree is read-only. */
const DENIED = new Set(["EPERM", "EACCES"]);

/** How hard a removal tries: how many times, and how long it waits before the next try, times the tries so far. */
interface Patience {
  readonly tries: number;
  readonly waitMs: number;
}

/**
 * This run's own folder: vitest ends its workers only after the run's teardown starts, so a file a worker holds lets go
 * within a moment. The waits add to 1 s.
 */
const OWN_FOLDER: Patience = { tries: 5, waitMs: 100 };
/**
 * An ended run's folder: one more try after 100 ms, for a tree made writable after a refusal for want of permission, or
 * a killed run's process that lets go a moment after it ends. A folder a running process still holds stays for the
 * next run, so no run's end waits on it.
 */
const ENDED_FOLDER: Patience = { tries: 2, waitMs: 100 };

/**
 * A run folder's owner: the pid of the vitest process the folder's name gives, where that pid means something, and when
 * the run last touched its owner file.
 */
interface Owner {
  readonly pid: number;
  readonly place: string;
  readonly touchedMs: number;
}

/** A folder to remove, with its owner when it is a run folder whose owner file was read. */
interface Folder {
  readonly path: string;
  readonly owner: Owner | undefined;
}

/**
 * Makes this run's folder, with its owner file, points the temp folder variables at it, and starts removing ended runs'
 * folders. The run's end removes its folder and waits for the others. A run whose folder cannot be made runs as runs
 * did before there was a run folder.
 */
export default async function setUpRunFolder(): Promise<() => Promise<void>> {
  const parent = runFolderParent();
  const folder = await makeRunFolder(parent);
  if (folder === undefined) return async () => undefined;
  const place = thisPlace();
  const owner: Owner = { pid: process.pid, place: place ?? "", touchedMs: Date.now() };
  await writeOwner(folder, owner);
  const stopHeartbeat = startHeartbeat(join(folder, OWNER_FILE), HEARTBEAT_MS);
  const ended = removeFolders(await endedFolders(parent, basename(folder), place), ENDED_FOLDER);
  const restoreVariables = pointVariablesAt(folder);
  return async () => {
    restoreVariables();
    await stopHeartbeat();
    const own: Folder = { path: folder, owner: { ...owner, touchedMs: Date.now() } };
    const [endedLeft, ownLeft] = await Promise.all([ended, removeFolders([own], OWN_FOLDER)]);
    warnLeft("could not remove the folders of ended test runs", endedLeft);
    warnLeft("could not remove its temporary folder", ownLeft);
  };
}

/**
 * Where this run's folder goes: the temp folder, unless the deepest Unix socket a test binds would not fit below a run
 * folder there, when `/tmp` (or the folder RERAIL_TEST_SHORT_TEMP names) takes its place. Tests, and the daemons they
 * start, listen on sockets below the temp folder, and a socket path holds at most 107 bytes on Linux and 103 on macOS:
 * macOS's per-user temp folder (`/var/folders/<2>/<28>/T`, 46 bytes) never leaves the room, and a TMPDIR set by hand
 * may not. Windows sockets are named pipes, with no such limit. A run started inside another run's folder stays there,
 * so that run's end removes what this one leaves.
 */
function runFolderParent(): string {
  const temp = tmpdir();
  if (process.platform === "win32" || temp.split(sep).some(part => RUN_FOLDER.test(part))) return temp;
  const deepestSocket = Buffer.byteLength(join(temp, `${PREFIX}${process.pid}-XXXXXX`)) + DEEPEST_SOCKET_BYTES;
  return deepestSocket <= SOCKET_PATH_BYTES ? temp : (process.env[SHORT_TEMP_VARIABLE] ?? SHORT_TEMP);
}

/**
 * Makes this run's folder in `parent`. Undefined, with one line on stderr, when `parent` refuses it, as a read-only
 * /tmp in a sandbox does: the run then leaves the temp folder variables as they are, so its tests make their folders
 * where they did before there was a run folder, and every test still runs.
 */
async function makeRunFolder(parent: string): Promise<string | undefined> {
  try {
    return await mkdtemp(join(parent, `${PREFIX}${process.pid}-`));
  } catch (error) {
    console.warn(`The test run could not make its folder in ${parent}, so its tests leave their folders in ${tmpdir()}: ${String(error)}`);
    return undefined;
  }
}

/**
 * Where this process's pid means something, as a run folder's owner file names it: the host, and on Linux the pid
 * namespace and the boot, since a container or sandbox with a pid namespace of its own may share the temp folder.
 * Undefined when this process cannot read its own namespace, so that no run folder's pid is checked.
 */
export function thisPlace(): string | undefined {
  const host = `host ${hostname()}\n`;
  if (process.platform !== "linux" && process.platform !== "android") return host;
  try {
    const boot = readFileSync("/proc/sys/kernel/random/boot_id", "utf8").trim();
    return `${host}pid namespace ${readlinkSync("/proc/self/ns/pid")}\nboot ${boot}\n`;
  } catch {
    return undefined;
  }
}

/**
 * Writes `owner` into the run folder at `folder`, touched when it says. A folder already gone, or one this user may not
 * write, keeps none: a later run then goes by the folder's age.
 */
async function writeOwner(folder: string, owner: Owner): Promise<void> {
  const file = join(folder, OWNER_FILE);
  const touched = new Date(owner.touchedMs);
  await writeFile(file, owner.place)
    .then(async () => await utimes(file, touched, touched))
    .catch(() => undefined);
}

/**
 * Touches `file` every `everyMs`, so that a run in another place sees this one still runs; returns what stops it, which
 * resolves once the touch in flight is done. The timer never keeps vitest's process alive.
 */
function startHeartbeat(file: string, everyMs: number): () => Promise<void> {
  let touches = Promise.resolve();
  const timer = setInterval(() => {
    touches = touches.then(async () => {
      const now = new Date();
      await utimes(file, now, now).catch(() => undefined);
    });
  }, everyMs);
  timer.unref();
  return async () => {
    clearInterval(timer);
    await touches;
  };
}

/** Points TMPDIR, TEMP and TMP at `folder`; returns what gives them back the values they had. */
function pointVariablesAt(folder: string): () => void {
  const prior = VARIABLES.map(name => [name, process.env[name]] as const);
  for (const name of VARIABLES) process.env[name] = folder;
  return () => {
    for (const [name, value] of prior) {
      if (value === undefined) delete process.env[name];
      else process.env[name] = value;
    }
  };
}

/** The folders in `parent` that no running test run uses, this run's own (named `own`) aside. */
async function endedFolders(parent: string, own: string, place: string | undefined): Promise<Folder[]> {
  const names = await readdir(parent).catch((): string[] => []);
  const paths = names.filter(name => name.startsWith(PREFIX) && name !== own).map(name => join(parent, name));
  const now = Date.now();
  const folders = await Promise.all(paths.map(async path => await endedFolder(path, place, now)));
  return folders.filter(folder => folder !== undefined);
}

/**
 * The folder at `path` when no running test run uses it, undefined while one may: a run folder goes by its owner file,
 * and any other `rerail-vitest-*` folder once it is a day old, a run folder whose run has not yet written its owner file
 * among them. A folder that cannot be read stays, and another user's folder, in a shared /tmp, is never this run's to
 * remove.
 */
async function endedFolder(path: string, place: string | undefined, now: number): Promise<Folder | undefined> {
  const info = await ownFolder(path);
  if (info === undefined) return undefined;
  const owner = await readOwner(path);
  const ended = owner === undefined ? now - info.mtimeMs > DAY_MS : runEnded(owner, place, now);
  return ended ? { path, owner } : undefined;
}

/** This user's folder at `path`; undefined for a file, a link, another user's folder or one that cannot be read. */
async function ownFolder(path: string): Promise<Stats | undefined> {
  const info = await lstat(path).catch(() => undefined);
  const uid = process.getuid?.();
  return info?.isDirectory() === true && (uid === undefined || info.uid === uid) ? info : undefined;
}

/** The owner of the run folder at `path`; undefined for a folder of another name, or a run folder with no owner file to read. */
async function readOwner(path: string): Promise<Owner | undefined> {
  const pid = RUN_FOLDER.exec(basename(path))?.[1];
  if (pid === undefined) return undefined;
  const file = join(path, OWNER_FILE);
  try {
    const [place, info] = await Promise.all([readFile(file, "utf8"), lstat(file)]);
    return { pid: Number(pid), place, touchedMs: info.mtimeMs };
  } catch {
    return undefined;
  }
}

/**
 * Whether a run has ended, by its folder's owner file. From this place its pid tells: the run ended once no process has
 * the pid, or this process does, which makes the folder an earlier process's. From another place the pid names some
 * other process or none here, so the run ended once its owner file went ten minutes untouched. A day untouched ends a
 * run from either place, since its pid may since belong to another process.
 */
function runEnded(owner: Owner, place: string | undefined, now: number): boolean {
  const untouched = now - owner.touchedMs;
  if (owner.place !== place) return untouched > QUIET_MS;
  return untouched > DAY_MS || owner.pid === process.pid || !isRunning(owner.pid);
}

/** Whether a process with `pid` runs. Signal 0 only checks; EPERM is another user's process. */
function isRunning(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === "EPERM";
  }
}

/** Removes each of `folders` with `patience`; returns why each one that stays did. */
async function removeFolders(folders: readonly Folder[], patience: Patience): Promise<string[]> {
  const left = await Promise.all(folders.map(async folder => await removeFolder(folder, patience)));
  return left.filter(reason => reason !== undefined);
}

/**
 * Removes `folder`; returns why it stays, or undefined once it is gone. When part of a run folder stays, its owner file
 * goes back as it was: the removal may have taken it with the rest, and without it a later run would keep the folder
 * for a day once the process holding it let go.
 */
async function removeFolder(folder: Folder, patience: Patience): Promise<string | undefined> {
  const left = await remove(folder.path, patience);
  if (left !== undefined && folder.owner !== undefined) await writeOwner(folder.path, folder.owner);
  return left;
}

/**
 * Removes `path`; returns why it stays, or undefined once it is gone. When the first try is refused for want of
 * permission, every folder in it is made writable before the next. While Windows refuses because a process holds a
 * file in it, it tries again as `patience` allows. Node's own `maxRetries` would retry at every level of the tree, so
 * one file held deep down would keep it trying for minutes.
 */
async function remove(path: string, patience: Patience): Promise<string | undefined> {
  for (let attempt = 1; ; attempt += 1) {
    try {
      await rm(path, { recursive: true, force: true });
      return undefined;
    } catch (error) {
      const code = (error as NodeJS.ErrnoException).code ?? "";
      if (!(await readyToRetry({ path, code, attempt, patience }))) return String(error);
    }
  }
}

/**
 * Whether the removal of `path`, refused with `code` at try `attempt`, gets another try, and readies it: after the
 * first refusal for want of permission it makes every folder in the tree writable, and it waits a little longer after
 * each try.
 */
async function readyToRetry(refusal: { path: string; code: string; attempt: number; patience: Patience }): Promise<boolean> {
  const { path, code, attempt, patience } = refusal;
  if (attempt >= patience.tries || !HELD.has(code)) return false;
  if (attempt === 1 && DENIED.has(code)) await unlock(path);
  await delay(patience.waitMs * attempt);
  return true;
}

/**
 * Gives this user every permission on each folder in the tree at `path`, so a removal can empty them. A test that makes
 * a folder read-only to see a write fail, killed before it gives the permission back, leaves one that `rm` cannot empty
 * on Linux and macOS (on Windows, Node's `rm` clears a read-only flag itself). Links are not followed, so nothing
 * outside the tree changes. A folder this user may not change (another user's) stays as it is, and the removal says why.
 */
async function unlock(path: string): Promise<void> {
  const info = await lstat(path).catch(() => undefined);
  if (info?.isDirectory() !== true) return;
  await chmod(path, 0o700).catch(() => undefined);
  const names = await readdir(path).catch(() => []);
  await Promise.all(names.map(async name => await unlock(join(path, name))));
}

/** Says on stderr what stayed, which the next run tries to remove again. */
function warnLeft(what: string, left: readonly string[]): void {
  if (left.length === 0) return;
  console.warn(`The test run ${what}; the next run tries again:\n${left.map(reason => `  ${reason}`).join("\n")}`);
}
