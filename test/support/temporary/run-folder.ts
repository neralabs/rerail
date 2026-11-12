/**
 * The run folder: one folder per vitest run that holds every temporary folder the run's tests make, and those of the
 * processes they start, so one removal when the run ends takes them all, a folder a test never removed included.
 * vitest.config.ts loads this module as a global setup, in vitest's own process before the first test file: it points
 * TMPDIR, TEMP and TMP at the folder, which `os.tmpdir()` reads, and the workers and every process they start inherit.
 *
 * A run killed before it ends (a closed terminal, a tool call's time limit) leaves its folder behind. The next run
 * removes it, once no process has the pid in its name. Folders that test runs made straight in the temp folder before
 * there was a run folder (`rerail-vitest-home-*`, `rerail-vitest-store-*`) go once they are a day old, since a
 * checkout on an older commit may still be running its tests in one. A run picks those folders at its start and
 * removes them while its tests run, so its first test file waits for none of them; its end waits for that removal.
 */
import { lstat, mkdtemp, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const PREFIX = "rerail-vitest-";
/** A run folder's name: the prefix, the pid of the vitest process that made it, then mkdtemp's six characters. */
const RUN_FOLDER = /^rerail-vitest-(\d+)-[A-Za-z0-9]{6}$/u;
const DAY_MS = 24 * 60 * 60 * 1000;
const VARIABLES = ["TMPDIR", "TEMP", "TMP"] as const;
const REMOVAL_TRIES = 5;
/** The errors Windows gives while a process still holds a file in a folder being removed. */
const HELD = new Set(["EBUSY", "EPERM", "ENOTEMPTY", "EACCES"]);

/**
 * Makes this run's folder and points the temp folder variables at it, and starts removing ended runs' folders, which
 * it picks before its own folder exists. The run's end removes its folder and waits for the others.
 */
export default async function setUpRunFolder(): Promise<() => Promise<void>> {
  const parent = runFolderParent();
  const ended = removeAll(await endedRuns(parent, Date.now()));
  const folder = await mkdtemp(join(parent, `${PREFIX}${process.pid}-`));
  const prior = VARIABLES.map(name => [name, process.env[name]] as const);
  for (const name of VARIABLES) process.env[name] = folder;
  return async () => {
    for (const [name, value] of prior) {
      if (value === undefined) delete process.env[name];
      else process.env[name] = value;
    }
    const [endedLeft, ownLeft] = await Promise.all([ended, removeAll([folder])]);
    warnLeft("could not remove the folders of ended test runs", endedLeft);
    warnLeft("could not remove its temporary folder", ownLeft);
  };
}

/**
 * Where run folders go: the temp folder, except macOS's per-user one (`/var/folders/<2>/<28>/T`, 46 bytes), where
 * `/tmp` takes its place. A daemon a test starts listens on a socket below the temp folder (`rerail-<uid>/<24 hex>
 * .sock`, or below the test's own folder), and macOS fits at most 104 bytes in a socket path: one folder more below
 * the per-user folder does not fit. A TMPDIR set by hand is used as it is.
 */
function runFolderParent(): string {
  const temp = tmpdir();
  return process.platform === "darwin" && temp.startsWith("/var/folders/") ? "/tmp" : temp;
}

/** The folders in `parent` that no running test run uses. */
async function endedRuns(parent: string, now: number): Promise<string[]> {
  const paths = (await readdir(parent)).filter(name => name.startsWith(PREFIX)).map(name => join(parent, name));
  const ended = await Promise.all(paths.map(async path => await isEnded(path, now)));
  return paths.filter((_, index) => ended[index]);
}

/**
 * Whether no running test run uses the folder at `path`: a run folder whose process has ended, or a folder of a
 * run from before run folders that is a day old. A folder named for this process's pid can only be an ended process's,
 * since this run picks the ended runs before it makes its own. A folder that cannot be read stays, and another user's
 * folder, in a shared /tmp, is never this run's to remove.
 */
async function isEnded(path: string, now: number): Promise<boolean> {
  const info = await lstat(path).catch(() => undefined);
  const uid = process.getuid?.();
  if (info === undefined || !info.isDirectory() || (uid !== undefined && info.uid !== uid)) return false;
  const pid = RUN_FOLDER.exec(basename(path))?.[1];
  if (pid === undefined) return now - info.mtimeMs > DAY_MS;
  return Number(pid) === process.pid || !isRunning(Number(pid));
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

/** Removes each of `paths`; returns why each one that stays did. */
async function removeAll(paths: readonly string[]): Promise<string[]> {
  const left = await Promise.all(paths.map(remove));
  return left.filter(reason => reason !== undefined);
}

/**
 * Removes `path`; returns why it stays, or undefined once it is gone. While Windows refuses because a process holds a
 * file in it, it tries again for a second and a half: vitest ends its workers only after the run's teardown starts,
 * and a killed run's processes let go a moment after they end. Node's own `maxRetries` would retry at every level of
 * the tree, so one file held deep down would keep it trying for minutes.
 */
async function remove(path: string): Promise<string | undefined> {
  for (let attempt = 1; ; attempt += 1) {
    try {
      await rm(path, { recursive: true, force: true });
      return undefined;
    } catch (error) {
      if (attempt === REMOVAL_TRIES || !HELD.has((error as NodeJS.ErrnoException).code ?? "")) return String(error);
      await delay(100 * attempt);
    }
  }
}

/** Says on stderr what stayed, which the next run tries to remove again. */
function warnLeft(what: string, left: readonly string[]): void {
  if (left.length === 0) return;
  console.warn(`The test run ${what}; the next run tries again:\n${left.map(reason => `  ${reason}`).join("\n")}`);
}
