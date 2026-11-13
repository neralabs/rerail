/**
 * The process `vacuumGivingWay` starts (`vacuum-giving-way.ts`): `VACUUM` on the database file named by its argument, on
 * a connection that waits for no other (`busy_timeout = 0`) and syncs its commit as every Rerail writer does
 * (`synchronous = FULL`). Before its `VACUUM` it takes the write lock and reads the file's pages (`saidHolding`): with
 * none free it runs no `VACUUM`, since another connection has given the space back; otherwise it writes the page count
 * on stdout. Exits 0 once `VACUUM` has committed or found no free page, `BUSY_EXIT` when another connection held the
 * write lock, and 1 with SQLite's error on stderr otherwise. Ended at any point before it commits, it leaves the file as
 * it was: SQLite ignores the pages of a transaction that never committed.
 *
 * The `VACUUM` runs on a worker thread, so the main thread stays free to read stdin, a pipe from the process that
 * started this one. That pipe closes when that process ends, however it ends (a stop, a kill, a crash), and this process
 * then ends at once, so a `VACUUM` that has not committed never holds the write lock past the daemon that started it.
 * Run on the main thread, a `VACUUM` ran to its end after its daemon was killed: on Linux a 6 s one held the lock 6.7 s
 * after the kill, and the restarted daemon's first write failed after its 5 s busy timeout.
 */
import { writeSync } from "node:fs";
import { isMainThread, parentPort, Worker, workerData } from "node:worker_threads";
import Database from "better-sqlite3";
import { BUSY_EXIT, HOLDING } from "./vacuum-giving-way.js";

function vacuum(path: string): void {
  const client = new Database(path, { fileMustExist: true });
  try {
    client.pragma("busy_timeout = 0");
    client.pragma("synchronous = FULL");
    if (saidHolding(client)) client.exec("VACUUM");
  } finally {
    client.close();
  }
}

/**
 * Whether the file has free pages to give back, read while this connection holds the write lock; when it has, writes
 * `HOLDING` and the file's page count on stdout before it lets the lock go. No other connection commits while this one
 * holds the lock, and the `VACUUM` asks for it again at once, failing with `BUSY_EXIT` when another connection holds it
 * then. So the page count falls below the one written when this process's `VACUUM` commits, and before that only if
 * another connection's `VACUUM` takes the lock and commits in the instant between the two, so it took no longer than
 * that instant. The attempt takes that fall as its commit (`committedBy`), whatever another connection's `VACUUM`
 * (`rerail scrub`, `rerail reset`) did before the read. The write is synchronous: the line is on the pipe before the
 * `VACUUM`, which holds this thread, begins.
 */
function saidHolding(client: Database.Database): boolean {
  client.exec("BEGIN IMMEDIATE");
  try {
    const free = pragmaNumber(client, "freelist_count") > 0;
    if (free) writeSync(1, `${HOLDING}${pragmaNumber(client, "page_count")}\n`);
    return free;
  } finally {
    client.exec("ROLLBACK");
  }
}

function pragmaNumber(client: Database.Database, name: string): number {
  // `simple` returns the first column of the first row: a number for these pragmas.
  return client.pragma(name, { simple: true }) as number;
}

/**
 * `vacuum` on `path`, its outcome as an exit code. The error goes straight to stderr: the worker thread's own stderr
 * passes through the main thread, which exits as soon as this thread ends.
 */
function vacuumExitCode(path: string): number {
  try {
    vacuum(path);
    return 0;
  } catch (error) {
    writeSync(2, error instanceof Error ? error.message : String(error));
    return error instanceof Database.SqliteError && error.code.startsWith("SQLITE_BUSY") ? BUSY_EXIT : 1;
  }
}

/** Ends this process at once, whatever its worker thread is doing: SQLite drops a `VACUUM` that has not committed. */
function endNow(): void {
  process.kill(process.pid, "SIGKILL");
}

/**
 * Runs `vacuumExitCode` on the file this process was started with, on a worker thread of this module, and exits with
 * its code; ends at once when stdin ends or fails, as it does when the process that started this one ends. The worker
 * takes this process's options, `tsx` among them where the tests run this module from its TypeScript source.
 */
function main(): void {
  const worker = new Worker(new URL(import.meta.url), { workerData: process.argv[2] ?? "" });
  worker.once("message", (code: number) => {
    process.exitCode = code;
  });
  worker.once("error", (error: unknown) => {
    writeSync(2, error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  });
  worker.once("exit", () => process.exit());
  process.stdin.once("end", endNow).once("error", endNow).resume();
}

if (isMainThread) main();
else parentPort?.postMessage(vacuumExitCode(String(workerData)));
