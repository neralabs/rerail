/**
 * `VACUUM` for the daemon's move of audit rule provenance (`rule-provenance-move.ts`), run so that it never holds up a
 * write on the thread that answers hooks. `VACUUM` holds the write lock until it ends, every Rerail writer waits for
 * that lock on its own thread for up to its 5 s busy timeout (`database.ts`), and the daemon's stores wait on the thread
 * that answers hooks: the `VACUUM` after 4,000 audits took 8.9 s on a loaded Windows machine. better-sqlite3 cannot stop
 * a statement once it runs, so the `VACUUM` runs in a process of its own (`vacuum-process.ts`), which this ends once the
 * watched thread has not answered for `GIVE_WAY_MS`: SQLite drops a `VACUUM` that has not committed, the lock is free,
 * this cuts the WAL back from the pages that `VACUUM` wrote (`cutWal`), and the write that waited goes through. Each
 * attempt starts once no other connection has committed to the file for a while, twice as long after each one that gave
 * way, so a busy hour costs a few short attempts, and the `VACUUM` runs in the first quiet moment long enough for it.
 */
import { spawn, type ChildProcess } from "node:child_process";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";
import type { MessagePort } from "node:worker_threads";
import type Database from "better-sqlite3";

/** How long the watched thread may go without answering before the `VACUUM` gives way: well under a hook's 2 s probe. */
export const GIVE_WAY_MS = 500;

/** How often the watch asks the watched thread whether it is answering. */
const PING_MS = 50;

/** How long no other connection may commit before the first attempt; each attempt that gives way doubles it. */
const FIRST_QUIET_MS = 2_000;

/** The longest quiet an attempt waits for. */
const LONGEST_QUIET_MS = 300_000;

/** How often the wait for a quiet file reads whether another connection has committed. */
const QUIET_POLL_MS = 250;

/**
 * How long, once it has ended a `VACUUM`'s process, the move tries to cut the WAL back (`cutWal`): the process can take
 * a few hundred milliseconds to end when its sync is under way, and it holds the lock until then.
 */
const CUT_WAL_MS = 2_000;

/** The exit code of a `vacuum-process.ts` that found another connection holding the write lock. */
export const BUSY_EXIT = 75;

/** The messages of the watch: the watching thread posts `PING`, and the watched thread answers each with `PONG`. */
export const PING = "rerail-vacuum-ping";
export const PONG = "rerail-vacuum-pong";

/**
 * Watches the thread whose writes a `VACUUM` gives way to, from the start of one attempt to its end: calls `stalled`
 * whenever that thread has not answered for `GIVE_WAY_MS`; returns what ends the watch.
 */
export type ThreadWatch = (stalled: () => void) => () => void;

/** The `VACUUM` stopped before it finished, because the move was asked to stop; the file is as it was. */
export class VacuumStopped extends Error {
  constructor() {
    super("Rerail stopped before VACUUM finished");
  }
}

/** One attempt: it committed, it gave way to a write or a stop, or another connection held the lock as it began. */
type Attempt = "vacuumed" | "gave-way" | "busy";

/**
 * The watch over the thread at the other end of `port`, which answers each `PING` with `PONG` (the daemon's thread,
 * `moveRuleProvenancesInWorker`). Pings go only while an attempt runs.
 */
export function watchThread(port: MessagePort): ThreadWatch {
  return stalled => {
    let answered = performance.now();
    const pong = (message: unknown): void => {
      if (message === PONG) answered = performance.now();
    };
    port.on("message", pong);
    const ping = setInterval(() => {
      if (performance.now() - answered > GIVE_WAY_MS) stalled();
      port.postMessage(PING);
    }, PING_MS);
    return () => {
      clearInterval(ping);
      port.off("message", pong);
      // As before the watch: the port alone keeps the thread alive no longer than the move does.
      port.unref();
    };
  };
}

/**
 * Runs `VACUUM` on the database at `path`, giving way to the thread `watch` watches, until it commits: each attempt waits
 * until no connection but `client`, the caller's, has committed for a while (`quietFor`), then runs in a process of its
 * own that ends once the thread stops answering (`vacuumOnce`), and says on stderr when the next one runs. Resolves once
 * the file holds no free page; throws `VacuumStopped` once `signal` stops it, and what `vacuum-process.ts` reported when
 * `VACUUM` failed.
 */
export async function vacuumGivingWay(
  client: Database.Database,
  path: string,
  { watch, signal }: { readonly watch: ThreadWatch; readonly signal: AbortSignal | undefined },
): Promise<void> {
  for (let quiet = FIRST_QUIET_MS; ; quiet = longerQuiet(quiet)) {
    if (!(await quietFor(client, quiet, signal))) throw new VacuumStopped();
    // An attempt ended just after it committed counts as one that ran.
    if (pragmaNumber(client, "freelist_count") === 0) return;
    if ((await vacuumOnce(client, path, { watch, signal })) === "vacuumed") return;
    if (signal?.aborted === true) throw new VacuumStopped();
    process.stderr.write(`rerail: ${path}: VACUUM gave way to another connection's write, and runs again once no other `
      + `connection has written to the file for ${longerQuiet(quiet) / 1_000} s.\n`);
  }
}

function longerQuiet(quiet: number): number {
  return Math.min(2 * quiet, LONGEST_QUIET_MS);
}

/**
 * Whether no connection but `client` committed to the file for `quietMs` (`PRAGMA data_version` changes on each other
 * connection's commit), read every `QUIET_POLL_MS`; false as soon as `signal` stops the wait.
 */
async function quietFor(client: Database.Database, quietMs: number, signal: AbortSignal | undefined): Promise<boolean> {
  let version = pragmaNumber(client, "data_version");
  let since = performance.now();
  while (performance.now() - since < quietMs) {
    await sleep(QUIET_POLL_MS, undefined, { signal }).catch(() => undefined);
    if (signal?.aborted === true) return false;
    const now = pragmaNumber(client, "data_version");
    if (now !== version) [version, since] = [now, performance.now()];
  }
  return true;
}

/**
 * One attempt in a process of its own, which a stall of the watched thread or a stop ends. Once it ends that process,
 * it cuts the WAL back on `client` (`cutWal`) before the write that waited takes the lock.
 */
async function vacuumOnce(
  client: Database.Database,
  path: string,
  { watch, signal }: { readonly watch: ThreadWatch; readonly signal: AbortSignal | undefined },
): Promise<Attempt> {
  const child = startVacuumProcess(path);
  let ended = false;
  const end = (): void => {
    if (ended) return;
    ended = true;
    child.kill("SIGKILL");
    cutWal(client);
  };
  const unwatch = watch(end);
  const unlisten = onAbort(signal, end);
  try {
    return await attemptOf(child, () => ended);
  } finally {
    unwatch();
    unlisten();
  }
}

/**
 * `vacuum-process.ts` on `path`: its JavaScript as the package ships it, or, where this module runs from its TypeScript
 * source as the tests run it, that source through `tsx`. No console window opens for it on Windows.
 */
function startVacuumProcess(path: string): ChildProcess {
  const fromSource = new URL(import.meta.url).pathname.endsWith(".ts");
  const entry = fileURLToPath(new URL(`./vacuum-process.${fromSource ? "ts" : "js"}`, import.meta.url));
  return spawn(process.execPath, [...(fromSource ? ["--import", "tsx"] : []), entry, path], {
    stdio: ["ignore", "ignore", "pipe"],
    windowsHide: true,
  });
}

/** Calls `run` once `signal` aborts, at once when it already has; returns what stops listening. */
function onAbort(signal: AbortSignal | undefined, run: () => void): () => void {
  if (signal === undefined) return () => undefined;
  if (signal.aborted) run();
  else signal.addEventListener("abort", run, { once: true });
  return () => signal.removeEventListener("abort", run);
}

/** How `child` ended: given way once `ended` says this process ended it, else by its exit code and stderr. */
function attemptOf(child: ChildProcess, ended: () => boolean): Promise<Attempt> {
  const errors: Buffer[] = [];
  child.stderr?.on("data", (chunk: Buffer) => errors.push(chunk));
  return new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("close", (code: number | null) => {
      const attempt = attemptByExit(ended(), code);
      if (attempt !== undefined) resolve(attempt);
      else reject(new Error(Buffer.concat(errors).toString().trim() || `the VACUUM process exited with code ${code}`));
    });
  });
}

function attemptByExit(ended: boolean, code: number | null): Attempt | undefined {
  if (ended) return "gave-way";
  if (code === 0) return "vacuumed";
  return code === BUSY_EXIT ? "busy" : undefined;
}

/**
 * Cuts the WAL back to nothing with a TRUNCATE checkpoint, tried every millisecond for up to `CUT_WAL_MS` while the
 * ended process still holds the lock or a read is in the way. A `VACUUM` ended before it committed leaves the pages it
 * wrote in the WAL, still unwritten to disk, and the next commit's sync writes them all out first: 2 s after a `VACUUM`
 * of 1 GB in use, on the daemon's thread when the write that waited is the daemon's. Truncating drops them unwritten.
 * Taking the lock within a millisecond of its release, this beats that write, whose busy handler tries again every
 * 100 ms; when it loses, that commit writes the pages out, as before.
 */
function cutWal(client: Database.Database): void {
  const until = performance.now() + CUT_WAL_MS;
  const pause = new Int32Array(new SharedArrayBuffer(4));
  while (!walCut(client) && performance.now() < until) Atomics.wait(pause, 0, 0, 1);
}

/** One TRUNCATE checkpoint: whether it emptied the WAL, or failed for a reason a retry does not fix. */
function walCut(client: Database.Database): boolean {
  try {
    const [checkpoint] = client.pragma("wal_checkpoint(TRUNCATE)") as { busy: number }[];
    return checkpoint?.busy === 0;
  } catch {
    // Not another connection in the way (that reports busy): the checkpoint after the next attempt empties the WAL.
    return true;
  }
}

function pragmaNumber(client: Database.Database, name: string): number {
  // `simple` returns the first column of the first row: a number for these pragmas.
  return client.pragma(name, { simple: true }) as number;
}
