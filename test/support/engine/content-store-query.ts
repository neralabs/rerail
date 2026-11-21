/**
 * Reading the engine's content store in a test, from a process of its own.
 *
 * The engine holds its store through the SQLite linked into its addon, and better-sqlite3 is another copy of SQLite.
 * Two copies in one process each keep the store's POSIX locks as if they were alone, and the kernel never reports a
 * process its own locks. So better-sqlite3, opening a store the engine has open, finds no other connection: it cuts the
 * wal-index (`-shm`) to 3 bytes under the engine's map of it, and an engine thread that reads the wal-index before the
 * file grows again dies of SIGBUS. Closing it then drops every lock this process holds on the store, the engine's too,
 * so the next process to open the store cuts the wal-index again and may checkpoint past the engine's readers (SQLite's
 * "How To Corrupt An SQLite Database File", §2.2.1). In another process SQLite's locks work as between any two.
 */
import { execFileSync } from "node:child_process";
import { createRequire } from "node:module";

const SCRIPT = [
  "const [driver, path, sql, parameters] = process.argv.slice(1);",
  "const Database = require(driver);",
  "const store = new Database(path, { readonly: true, fileMustExist: true });",
  "try { process.stdout.write(JSON.stringify(store.prepare(sql).all(...JSON.parse(parameters)))); } finally { store.close(); }",
].join("\n");

const driver = createRequire(import.meta.url).resolve("better-sqlite3");

/** The rows `sql` reads, with `parameters` bound, from the content store at `storePath`, read in a child process, read-only. */
export function queryContentStore<Row = Record<string, unknown>>(
  storePath: string,
  sql: string,
  parameters: readonly (string | number | null)[] = [],
): Row[] {
  const output = execFileSync(process.execPath, ["-e", SCRIPT, driver, storePath, sql, JSON.stringify(parameters)], { encoding: "utf8" });
  return JSON.parse(output) as Row[];
}
