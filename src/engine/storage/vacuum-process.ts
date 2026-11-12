/**
 * The process `vacuumGivingWay` starts (`vacuum-giving-way.ts`): `VACUUM` on the database file named by its argument, on
 * a connection that waits for no other (`busy_timeout = 0`) and syncs its commit as every Rerail writer does
 * (`synchronous = FULL`). Exits 0 once `VACUUM` has committed, `BUSY_EXIT` when another connection held the write lock,
 * and 1 with SQLite's error on stderr otherwise. Ended at any point before it commits, it leaves the file as it was:
 * SQLite ignores the pages of a transaction that never committed.
 */
import Database from "better-sqlite3";
import { BUSY_EXIT } from "./vacuum-giving-way.js";

function vacuum(path: string): void {
  const client = new Database(path, { fileMustExist: true });
  try {
    client.pragma("busy_timeout = 0");
    client.pragma("synchronous = FULL");
    client.exec("VACUUM");
  } finally {
    client.close();
  }
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
