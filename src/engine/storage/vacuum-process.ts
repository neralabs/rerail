/**
 * The process `vacuumGivingWay` starts (`vacuum-giving-way.ts`): `VACUUM` on the database file named by its argument, on
 * a connection that waits for no other (`busy_timeout = 0`) and syncs its commit as every Rerail writer does
 * (`synchronous = FULL`). Before its `VACUUM` it takes the write lock and reads the file's pages (`saidHolding`): with
 * none free it runs no `VACUUM`, since another connection has given the space back; otherwise it writes the page count
 * on stdout. Exits 0 once `VACUUM` has committed or found no free page, `BUSY_EXIT` when another connection held the
 * write lock, and 1 with SQLite's error on stderr otherwise. Ended at any point before it commits, it leaves the file as
 * it was: SQLite ignores the pages of a transaction that never committed.
 */
import { writeSync } from "node:fs";
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
 * holds the lock, and the `VACUUM` asks for it again at once, failing with `BUSY_EXIT` when another connection took it
 * in between. So the page count falls below the one written only once this process's `VACUUM` commits: the attempt
 * takes that fall, and no other, as its commit, whatever another connection's `VACUUM` (`rerail scrub`, `rerail reset`)
 * did before. The write is synchronous: the line is on the pipe before the `VACUUM`, which holds this thread, begins.
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

/** `vacuum` on the file this process was started with, its outcome as the exit code. */
function main(): void {
  try {
    vacuum(process.argv[2] ?? "");
  } catch (error) {
    const busy = error instanceof Database.SqliteError && error.code.startsWith("SQLITE_BUSY");
    process.stderr.write(error instanceof Error ? error.message : String(error));
    process.exitCode = busy ? BUSY_EXIT : 1;
  }
}

main();
