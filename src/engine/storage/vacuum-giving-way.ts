/**
 * `VACUUM` for the daemon's move of audit rule provenance (`rule-provenance-move.ts`), run so that it never holds up a
 * write on the thread that answers hooks. `VACUUM` holds the write lock until it commits, every Rerail writer waits for
 * that lock on its own thread for up to its 5 s busy timeout (`database.ts`), and the daemon's stores wait on the thread
 * that answers hooks: the `VACUUM` after 4,000 audits took 8.9 s on a loaded Windows machine. better-sqlite3 cannot stop
 * a statement once it runs, so the `VACUUM` runs in a process of its own (`vacuum-process.ts`), which this ends once the
 * watched thread has not answered for `GIVE_WAY_MS` while the `VACUUM` has not committed: SQLite drops a `VACUUM` that
 * has not committed, the lock is free, this cuts the WAL back from the pages that `VACUUM` wrote (`cutWal`), and the
 * write that waited goes through. Each attempt starts once no other connection has committed to the file for a while,
 * twice as long after each one that gave way, up to `LONGEST_QUIET_MS`, so the `VACUUM` runs in the first quiet moment
 * long enough for it; once it has waited `noticeMs` it says so, once, with what its attempts did meanwhile.
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

/**
 * The longest quiet an attempt waits for. On one day of tool-call times from a machine running many coding agents, a
 * 10 s `VACUUM` started at a single session's busiest hour committed within 10.3 minutes with this cap, against 43
 * minutes with a 60 s cap and 2.9 hours with 5 minutes, after at most 5 attempts that gave way, each costing a hook
 * about 0.5 s.
 */
const LONGEST_QUIET_MS = 30_000;

/**
 * How long the `VACUUM` may wait for a quiet file before it says so, once. On the same day, with every session writing
 * into one daemon, a 5 s `VACUUM` waited 1.9 hours at the median; with a single session's writes, even a 10 s one
 * waited 10.3 minutes at most.
 */
export const WAIT_NOTICE_MS = 3_600_000;

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

/** How `vacuumGivingWay` gives way, and to what. */
export interface GivingWay {
  readonly watch: ThreadWatch;
  /** Stops the wait, and ends a running attempt at once. */
  readonly signal: AbortSignal | undefined;
  /** Why the next attempt may not run (too little free disk), read before each one; undefined when it may. */
  readonly blocked: () => string | undefined;
  /** Called once if no attempt has committed `noticeMs` after the first wait began, with what the attempts did. */
  readonly waitedLong: (waiting: Waiting) => void;
  readonly noticeMs: number;
}

/**
 * What the attempts of a `VACUUM` that gives way did from the first wait on: how many gave way (to the watched thread,
 * or to another connection that held the lock), and whether the next waits for its quiet or one is running.
 */
export type Waiting =
  | { readonly kind: "waiting"; readonly gaveWay: number; readonly quietMs: number }
  | { readonly kind: "running"; readonly gaveWay: number };

/** The attempts so far, as `vacuumGivingWay` counts them. */
interface Attempts {
  gaveWay: number;
  /** The quiet the next attempt waits for. */
  quietMs: number;
  running: boolean;
}

/** The `VACUUM` did not run to its end, for the reason its message gives; the file is as it was. */
export class VacuumNotRun extends Error {}

/** The reason a stop gives. */
export const STOPPED = "Rerail stopped before VACUUM finished";

/**
 * One attempt: it gave the free pages back (or found none left), it gave way to the watched thread or a stop, or
 * another connection held the lock as it began.
 */
type Attempt = "shrunk" | "gave-way" | "busy";

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
 * Runs `VACUUM` on the database at `path`, giving way to the thread `way.watch` watches, until it commits: each attempt
 * waits until no connection but `client`, the caller's, has committed for a while (`quietFor`), then runs in a process
 * of its own that ends once the thread stops answering (`vacuumOnce`), and stderr says when the next one runs. Resolves
 * once the file holds no free page; throws `VacuumNotRun` once `way.signal` stops it or `way.blocked` gives a reason, and
 * what `vacuum-process.ts` reported when `VACUUM` failed.
 */
export async function vacuumGivingWay(client: Database.Database, path: string, way: GivingWay): Promise<void> {
  const attempts: Attempts = { gaveWay: 0, quietMs: FIRST_QUIET_MS, running: false };
  const notice = setTimeout(() => way.waitedLong(waitingOf(attempts)), way.noticeMs);
  try {
    for (;;) {
      const attempt = await nextAttempt(client, path, way, attempts);
      if (attempt === "shrunk") return;
      attempts.gaveWay += 1;
      attempts.quietMs = Math.min(2 * attempts.quietMs, LONGEST_QUIET_MS);
      process.stderr.write(notRunLine(path, attempt, attempts.quietMs));
    }
  } finally {
    clearTimeout(notice);
  }
}

/** What `attempts` did so far, as `GivingWay.waitedLong` hears it. */
function waitingOf({ gaveWay, quietMs, running }: Attempts): Waiting {
  return running ? { kind: "running", gaveWay } : { kind: "waiting", gaveWay, quietMs };
}

/**
 * Waits for the quiet `attempts` names, then runs one attempt unless `way` stops or blocks it, or no free page is left
 * to give back; `attempts` says it runs until it ends.
 */
async function nextAttempt(client: Database.Database, path: string, way: GivingWay, attempts: Attempts): Promise<Attempt> {
  if (!(await quietFor(client, attempts.quietMs, way.signal))) throw new VacuumNotRun(STOPPED);
  const blocked = way.blocked();
  if (blocked !== undefined) throw new VacuumNotRun(blocked);
  if (pragmaNumber(client, "freelist_count") === 0) return "shrunk";
  attempts.running = true;
  try {
    return stoppedOr(await vacuumOnce(client, path, way), way.signal);
  } finally {
    attempts.running = false;
  }
}

/** `attempt`, unless a stop ended it before it committed. */
function stoppedOr(attempt: Attempt, signal: AbortSignal | undefined): Attempt {
  if (attempt !== "shrunk" && signal?.aborted === true) throw new VacuumNotRun(STOPPED);
  return attempt;
}

/** The line for an attempt that did not commit, with why, and how long the next one waits. */
function notRunLine(path: string, attempt: "gave-way" | "busy", quietMs: number): string {
  const why = attempt === "busy"
    ? "another connection held the write lock as it began"
    : `the daemon's thread did not answer for ${GIVE_WAY_MS / 1_000} s, as when a write of its own waits for that lock`;
  return `rerail: ${path}: VACUUM gave way (${why}), and runs again once no other connection has written to the file `
    + `for ${quietMs / 1_000} s.\n`;
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
 * One attempt in a process of its own, which a stop ends, and a stall of the watched thread too until the `VACUUM` has
 * committed (`attemptEnd`). Ended before it committed, it cuts the WAL back on `client` (`cutWal`) before the write that
 * waited takes the lock.
 */
async function vacuumOnce(client: Database.Database, path: string, { watch, signal }: GivingWay): Promise<Attempt> {
  const committed = committedSince(client);
  const child = startVacuumProcess(path);
  const attempt = attemptEnd(child, client, committed);
  const unwatch = watch(() => attempt.end(false));
  const unlisten = onAbort(signal, () => attempt.end(true));
  try {
    const { code, errors } = await exitOf(child);
    const outcome = attemptOf(committed(), attempt.ended(), code);
    if (outcome === undefined) throw new Error(errors || `the VACUUM process exited with code ${code}`);
    return outcome;
  } finally {
    unwatch();
    unlisten();
  }
}

/**
 * Whether a `VACUUM` has committed since this was called. It gives the free pages back, so the file's page count falls,
 * which no other write does; a `VACUUM` that has not committed leaves it as it was.
 */
function committedSince(client: Database.Database): () => boolean {
  const pages = pragmaNumber(client, "page_count");
  return () => pragmaNumber(client, "page_count") < pages;
}

/**
 * What ends an attempt early, and whether it did. A stop always ends it. A stall of the watched thread ends it only
 * while the `VACUUM` has not committed: once it has, it holds the write lock no more, its process only writes the WAL
 * into the file without that lock, and whatever the thread waits for is not the `VACUUM`. Ending kills the process,
 * then cuts the WAL back (`cutWal`).
 */
function attemptEnd(
  child: ChildProcess,
  client: Database.Database,
  committed: () => boolean,
): { readonly end: (stopping: boolean) => void; readonly ended: () => boolean } {
  let ended = false;
  return {
    end: stopping => {
      if (ended || (!stopping && committed())) return;
      ended = true;
      child.kill("SIGKILL");
      cutWal(client, committed);
    },
    ended: () => ended,
  };
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

/** How `child` ended: its exit code, and what it wrote to stderr. */
function exitOf(child: ChildProcess): Promise<{ code: number | null; errors: string }> {
  const errors: Buffer[] = [];
  child.stderr?.on("data", (chunk: Buffer) => errors.push(chunk));
  return new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("close", (code: number | null) => resolve({ code, errors: Buffer.concat(errors).toString().trim() }));
  });
}

/** An attempt by whether its `VACUUM` committed, whether a stall or stop ended it, and its process's exit code. */
function attemptOf(committed: boolean, ended: boolean, code: number | null): Attempt | undefined {
  if (committed || code === 0) return "shrunk";
  if (ended) return "gave-way";
  return code === BUSY_EXIT ? "busy" : undefined;
}

/**
 * Cuts the WAL back to nothing with a TRUNCATE checkpoint, tried every millisecond for up to `CUT_WAL_MS` while the
 * ended process still holds the lock or a read is in the way. A `VACUUM` ended before it committed leaves the pages it
 * wrote in the WAL, still unwritten to disk, and the next commit's sync writes them all out first: 2 s after a `VACUUM`
 * of 1 GB in use, on the daemon's thread when the write that waited is the daemon's. Truncating drops them unwritten.
 * Taking the lock within a millisecond of its release, this beats that write, whose busy handler tries again every
 * 100 ms; when it loses, that commit writes the pages out, as before. It never truncates once the `VACUUM` turns out to
 * have committed: the WAL then holds the rewritten file, which a TRUNCATE checkpoint would copy into the database file
 * while it holds the write lock. A PASSIVE checkpoint copies it instead, holding no write lock, so no other
 * connection's commit makes its own checkpoint copy it on its thread.
 */
function cutWal(client: Database.Database, committed: () => boolean): void {
  const until = performance.now() + CUT_WAL_MS;
  const pause = new Int32Array(new SharedArrayBuffer(4));
  while (!committed() && !walCut(client) && performance.now() < until) Atomics.wait(pause, 0, 0, 1);
  if (committed()) checkpointQuietly(client, "PASSIVE");
}

/** One TRUNCATE checkpoint: whether it emptied the WAL, or failed for a reason a retry does not fix. */
function walCut(client: Database.Database): boolean {
  return checkpointQuietly(client, "TRUNCATE");
}

/** One checkpoint in `mode`: whether no other connection kept it from finishing; true when it failed otherwise. */
function checkpointQuietly(client: Database.Database, mode: "PASSIVE" | "TRUNCATE"): boolean {
  try {
    const [checkpoint] = client.pragma(`wal_checkpoint(${mode})`) as { busy: number }[];
    return checkpoint?.busy === 0;
  } catch {
    // Not another connection in the way (that reports busy): the checkpoint after the move empties the WAL.
    return true;
  }
}

function pragmaNumber(client: Database.Database, name: string): number {
  // `simple` returns the first column of the first row: a number for these pragmas.
  return client.pragma(name, { simple: true }) as number;
}
