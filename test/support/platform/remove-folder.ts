/**
 * Removing a folder a test made: every test removes its folders through this module (the `test-folder-removal` rule of
 * `npm run check:owned-builtins`).
 *
 * Windows refuses to remove a folder while a process runs in it or holds a file in it (`EBUSY`, `EPERM`). A process
 * this test process started that still runs in the folder (a Git, a daemon, a hook) was left running by the test or by
 * the code under test, so the removal fails at once and names it, on every platform, where a retry would only wait for
 * it to end by chance. Any other holder is out of the test's reach (antivirus, the search indexer, a process another
 * process started, a handle the system releases late) and lets go within a moment, so a refused removal is retried as
 * Rerail's own removals are ({@link REMOVAL_RETRIES}). A holder that never lets go, most often a database or file this
 * test process left open, fails the removal once the retries run out, naming the file it holds.
 */
import { ChildProcess } from "node:child_process";
import { realpathSync, rmSync } from "node:fs";
import { rm } from "node:fs/promises";
import { basename, dirname, join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";

/**
 * Rerail's own `FOLDER_REMOVAL_RETRIES` (`src/engine/file-replace.ts`), repeated: the setup file loads this module
 * before every test file, and a module a setup file loads is cached before the test file's `vi.mock` calls apply, so
 * this one loads no Rerail module (the engine's `index.ts`, the only part of it test/support may import, is the whole
 * engine). `test/engine/file-replace.test.ts` holds the two equal.
 */
export const REMOVAL_RETRIES = { maxRetries: 10, retryDelay: 100 } as const;

/** A process this test process started, with the folder it runs in. */
interface Started {
  readonly child: ChildProcess;
  readonly cwd: string;
}

/** `ChildProcess#spawn`, which every start in node:child_process goes through; Node's types leave it out. */
type Spawn = (this: ChildProcess, options: { cwd?: string | URL | null | undefined }) => unknown;

const STARTED = Symbol.for("rerail.test.started-processes");

/**
 * The processes this test process started through node:child_process (`spawn`, `execFile`, `exec`, `fork`) that have
 * not exited. `ChildProcess#spawn` is wrapped once per process, and the set kept on `globalThis`, so the setup file's
 * copy of this module and a test file's see the same processes.
 */
function startedProcesses(): Set<Started> {
  const registry = globalThis as { [STARTED]?: Set<Started> };
  const existing = registry[STARTED];
  if (existing !== undefined) return existing;
  const started = new Set<Started>();
  registry[STARTED] = started;
  // Node's types declare no ChildProcess#spawn, the method every start calls on the process it creates.
  const prototype = ChildProcess.prototype as unknown as { spawn: Spawn };
  const spawn = prototype.spawn;
  prototype.spawn = function (this: ChildProcess, options) {
    const result = spawn.call(this, options);
    // A process that failed to start has no pid and never exits.
    if (this.pid !== undefined) {
      const entry = { child: this, cwd: startedIn(options.cwd) };
      started.add(entry);
      this.once("exit", () => started.delete(entry));
    }
    return result;
  };
  return started;
}
startedProcesses();

/** The folder a process starts in: `cwd` as `spawn` passes it on (null from `execFile` without one), else this process's. */
function startedIn(cwd: string | URL | null | undefined): string {
  if (cwd instanceof URL) return fileURLToPath(cwd);
  if (typeof cwd === "string") return resolve(cwd);
  try {
    return process.cwd();
  } catch {
    // The working folder was deleted under the process.
    return "";
  }
}

/** `path` as it is compared: Windows paths in any case and with either separator. */
function spelling(path: string): string {
  return process.platform === "win32" ? path.replaceAll("/", "\\").toLowerCase() : path;
}

/** Whether `text` names `folder` or a path inside it. */
function names(text: string, folder: string): boolean {
  const [haystack, needle] = [spelling(text), spelling(folder)];
  for (let at = haystack.indexOf(needle); at !== -1; at = haystack.indexOf(needle, at + 1)) {
    const next = haystack[at + needle.length];
    if (next === undefined || /[\\/"'\s]/u.test(next)) return true;
  }
  return false;
}

/**
 * `folder` as given and with the links and short names of the folders above it resolved (macOS's `/var` is
 * `/private/var`), while they exist. A link at `folder` itself is not followed: removing it leaves its target alone.
 */
function spellings(folder: string): string[] {
  try {
    return [...new Set([folder, join(realpathSync.native(dirname(folder)), basename(folder))])];
  } catch {
    return [folder];
  }
}

/**
 * The processes this test process started that still run in `folder` or name it, or a path in it, among their arguments
 * (`git -C <folder>`, `--state-file <folder>/daemon.json`), each as its pid and command line.
 */
export function runningIn(folder: string): string[] {
  const forms = spellings(folder);
  return [...startedProcesses()]
    .filter(({ child, cwd }) => child.exitCode === null && child.signalCode === null
      && forms.some(form => names(cwd, form) || child.spawnargs.some(arg => names(arg, form))))
    .map(({ child }) => `pid ${child.pid}: ${child.spawnargs.join(" ")}`);
}

/** The codes of a removal Windows refused because something holds the folder, the ones `rm` retries. */
const HELD = new Set(["EBUSY", "EPERM", "ENOTEMPTY", "EMFILE", "ENFILE"]);

function isHeld(error: unknown): error is NodeJS.ErrnoException {
  return error instanceof Error && "code" in error && HELD.has(String(error.code));
}

/** Throws, naming them, when processes this test process started still run in `path`. */
function assertNothingRunningIn(path: string): void {
  const running = runningIn(path);
  if (running.length === 0) return;
  throw new Error(
    `${path} is not removed while a process this test started still runs in it: ${running.join("; ")}. ` +
    "Close what started it, or wait for it to exit, before the folder goes.",
  );
}

/** Rethrows `error` unless it says something held the folder and `retry` allows another try. */
function assertRetryable(error: unknown, retry: boolean): asserts error is NodeJS.ErrnoException {
  if (!isHeld(error) || !retry) throw error;
}

/** The error a removal of `path` gives up with once {@link REMOVAL_RETRIES} run out: `error`, the last try's. */
function stillHeld(path: string, error: NodeJS.ErrnoException): Error {
  return new Error(
    `${path} is not removed: ${error.path ?? path} is still held (${error.code}) after ${REMOVAL_RETRIES.maxRetries} ` +
    "retries, and no process this test started runs in it. Close what the test opened there (a database, a file " +
    "handle, a watcher) before the folder goes; otherwise another program holds it (antivirus, the search indexer).",
    { cause: error },
  );
}

export interface RemoveFolderOptions {
  /** False: one try only, for a test that shows nothing holds the folder any more. True by default. */
  retry?: boolean;
}

/**
 * Removes the folder `path` and everything in it, if it exists. Fails at once while a process this test started still
 * runs in it ({@link runningIn}); retries while anything else holds it, and gives up after {@link REMOVAL_RETRIES},
 * naming the file still held.
 *
 * The tries are counted here, each one `rm` with no retries of its own: node:fs/promises' `rm` applies `maxRetries` at
 * every level of the tree, each try at a folder removing its contents with a full set of retries again, so one file
 * held directly in the folder keeps it trying for 67 s rather than 5.5 s, past the 60 s hook timeout.
 */
export async function removeFolder(path: string, options: RemoveFolderOptions = {}): Promise<void> {
  const retries = options.retry === false ? 0 : REMOVAL_RETRIES.maxRetries;
  for (let attempt = 0; !(await removed(path, attempt, retries)); attempt += 1) {
    // As `rm` waits between its own retries: 100 ms, then each 100 ms longer than the last.
    await delay((attempt + 1) * REMOVAL_RETRIES.retryDelay);
  }
}

/** Try `attempt` (from 0) of `retries` at removing `path`: true once it is gone, false while a holder leaves a retry. */
async function removed(path: string, attempt: number, retries: number): Promise<boolean> {
  assertNothingRunningIn(path);
  try {
    await rm(path, { recursive: true, force: true });
    return true;
  } catch (error) {
    assertRetryable(error, retries > 0);
    if (attempt === retries) throw stillHeld(path, error);
    return false;
  }
}

/**
 * {@link removeFolder} for code that cannot wait: a synchronous hook, a `finally` in a synchronous test. `rmSync` retries
 * the folder as a whole rather than each level, so it keeps its own retries.
 */
export function removeFolderSync(path: string, options: RemoveFolderOptions = {}): void {
  assertNothingRunningIn(path);
  try {
    rmSync(path, { recursive: true, force: true });
    return;
  } catch (error) {
    assertRetryable(error, options.retry ?? true);
  }
  rmSync(path, { recursive: true, force: true, ...REMOVAL_RETRIES });
}
