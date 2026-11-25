import { appendFileSync, existsSync, readFileSync, realpathSync } from "node:fs";
import { resolve } from "node:path";
import fsPromises, { type FileHandle } from "node:fs/promises";
import { syncBuiltinESMExports } from "node:module";
import Database from "better-sqlite3";

// What a power cut keeps of a write-ahead log beyond what a checkpoint copied: everything a sync of the log put on the
// disk. A process records its syncs of a database's log once `recordLogSyncs` ran in it: a sync of the log file
// through Node (`syncHistoryLog` opens it, syncs it and closes it), and a commit that wrote on a connection set to
// `synchronous = FULL` or above, which SQLite ends by syncing the log. Each record is the log's length at the sync's
// start, with the salts of the log's header then: a log a checkpoint started again carries other salts, and a sync of
// an earlier one keeps nothing of it. Set RERAIL_TEST_LOG_SYNCS to a file to record a child process's syncs there.

interface LogSync {
  readonly path: string;
  readonly length: number;
  readonly salts: string;
}

const recorded: LogSync[] = [];
let recording = false;

/** The log's length and its header's salts, or undefined while the database has no log. */
function logState(databasePath: string): Omit<LogSync, "path"> | undefined {
  if (!existsSync(`${databasePath}-wal`)) return undefined;
  const log = readFileSync(`${databasePath}-wal`);
  return { length: log.length, salts: log.length >= 24 ? log.subarray(16, 24).toString("hex") : "" };
}

/** One spelling of a database's path, whichever the product and the test spelt (a temp folder behind a link, say). */
function canonical(databasePath: string): string {
  return existsSync(databasePath) ? realpathSync.native(databasePath) : resolve(databasePath);
}

function record(databasePath: string, state: Omit<LogSync, "path"> | undefined): void {
  if (state === undefined) return;
  const sync = { path: canonical(databasePath), ...state };
  recorded.push(sync);
  const file = process.env.RERAIL_TEST_LOG_SYNCS;
  if (file !== undefined) appendFileSync(file, `${JSON.stringify(sync)}\n`);
}

/** The bytes of the log of `databasePath` a power cut keeps by the syncs recorded here and in `file`: 0 when none. */
export function syncedLength(databasePath: string, file?: string): number {
  const now = logState(databasePath);
  const fromFile = file === undefined || !existsSync(file)
    ? []
    : readFileSync(file, "utf8").split("\n").filter(Boolean).map(line => JSON.parse(line) as LogSync);
  const path = canonical(databasePath);
  const kept = [...recorded, ...fromFile].filter(sync => sync.path === path && sync.salts === now?.salts);
  return Math.max(0, ...kept.map(sync => sync.length));
}

const WRITES = /^\s*(insert|update|delete|replace|create|drop|alter)\b/iu;

/** Records each commit that wrote on a connection whose commits sync the log (`synchronous` FULL or EXTRA). */
function recordDurableCommits(): void {
  const dirty = new WeakSet<Database.Database>();
  // Whether the statement just run ended a transaction that wrote, or wrote outside one.
  const committedAWrite = (database: Database.Database, inTransactionBefore: boolean, source: string): boolean => {
    if (database.inTransaction) {
      if (WRITES.test(source)) dirty.add(database);
      return false;
    }
    const wrote = inTransactionBefore ? dirty.has(database) : WRITES.test(source);
    dirty.delete(database);
    return wrote;
  };
  const ended = (database: Database.Database, inTransactionBefore: boolean, source: string): void => {
    if (!committedAWrite(database, inTransactionBefore, source) || database.memory) return;
    if ((database.pragma("synchronous", { simple: true }) as number) >= 2) record(database.name, logState(database.name));
  };
  const probe = new Database(":memory:");
  const statement = Object.getPrototypeOf(probe.prepare("SELECT 1")) as Record<"run" | "get" | "all", (...args: unknown[]) => unknown>;
  probe.close();
  for (const method of ["run", "get", "all"] as const) {
    const original = statement[method];
    statement[method] = function (this: Database.Statement, ...args: unknown[]) {
      const before = this.database.inTransaction;
      const result = original.apply(this, args);
      ended(this.database, before, this.source);
      return result;
    };
  }
  const exec = Database.prototype.exec;
  Database.prototype.exec = function (this: Database.Database, source: string) {
    const before = this.inTransaction;
    const result = exec.call(this, source);
    ended(this, before, source);
    return result;
  };
}

/** Records each sync of a log Node makes: a handle opened on a `-wal` file, synced. */
function recordNodeSyncs(): void {
  const open = fsPromises.open;
  fsPromises.open = async function (...args: Parameters<typeof open>): Promise<FileHandle> {
    const handle = await open(...args);
    const path = String(args[0]);
    if (!path.endsWith("-wal")) return handle;
    const sync = handle.sync.bind(handle);
    handle.sync = async () => {
      const state = logState(path.slice(0, -"-wal".length));
      await sync();
      record(path.slice(0, -"-wal".length), state);
    };
    return handle;
  };
  // `import { open } from "node:fs/promises"` reads the patched function only once the builtin's exports are synced.
  syncBuiltinESMExports();
}

/** Starts recording this process's syncs of every database's log; once per process. */
export function recordLogSyncs(): void {
  if (recording) return;
  recording = true;
  recordDurableCommits();
  recordNodeSyncs();
}
