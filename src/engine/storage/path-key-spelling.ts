/**
 * The rows Rerail keys by a folder's path, copied to the key this build gives that folder. Builds before Windows keys
 * went through `realpathSync.native` stored the spelling a caller passed (`c:\work\app` from VS Code, a folder outside
 * Git as it was typed), and this build looks each folder up by the spelling the file system stores. Without the copy, a
 * checkout's start state and settings, its audit settings and audit, and its presentation preferences read as unset
 * after the update. The rows under the earlier spelling stay as they were, so an earlier build that shares the file
 * (after a downgrade, or a coding agent's hooks from an older install) still reads all of them; `path_key_spellings`
 * records each spelling that moved, which this build's lists leave out (`notAnEarlierSpelling` in `schema.ts`). The
 * `path-keys-as-stored` step (`migrations.ts`) runs it once per file, on Windows only: it reads the keys and looks each
 * up before the open takes the write lock (`lookUpPathKeys`), then, under the lock, writes a few rows per moved folder
 * (`respellPathKeys`). It never rewrites the audit history, and no lookup on disk runs under the lock. An open looks up
 * only keys on this computer's system drive and on the drive or share of the folder its caller works in
 * (`withDriveBudget`); the others wait for an open made from a folder on their drive (`openRerailDatabaseIn`).
 */
import { existsSync } from "node:fs";
import { resolve, win32 } from "node:path";
import { sql } from "drizzle-orm";
import { physicalPath } from "../catalog/checkout-control.js";
import type { RerailDatabase } from "./database.js";
import { holdsTable } from "./table-probe.js";

/**
 * The key this build gives a folder an earlier build keyed as a path, or undefined while its drive is not one this open
 * reads, or past its time in this open (`withDriveBudget`).
 */
export type PathKeyOf = (stored: string) => string | undefined;

/**
 * What the step looked up before the open took the write lock: each key with the key this build gives its folder, and
 * how far it read the audit history, so the step under the lock reads only the rows written since.
 */
export interface PathKeyLookups {
  /** Each key looked up; undefined for one that waits for a later open (`PathKeyOf`). */
  readonly keys: ReadonlyMap<string, string | undefined>;
  /** The last rowid of each audit history table as it was read; a table not listed is read whole under the lock. */
  readonly historyThrough: ReadonlyMap<string, number>;
}

/**
 * Whether a key an earlier build stored can name its folder otherwise than this build does: on Windows only, where keys
 * go through `realpathSync.native` (`checkoutRealpath`). On macOS and Linux the realpath did not change, so the step
 * reads and writes nothing there.
 */
export function pathKeysCanMove(platform: NodeJS.Platform = process.platform): boolean {
  return platform === "win32";
}

/** A column that holds a path, its names written out here, as a migration step's are, so later schema changes leave it. */
interface PathColumn {
  readonly table: string;
  /** The column that holds the path. */
  readonly key: string;
  /** The rows whose key is a path, as an SQL condition; the table's other rows are left as they are. */
  readonly paths: string;
}

/** A table keyed by a path. */
interface PathKeyedTable extends PathColumn {
  /** The other primary key columns: two rows that come to one path clash only when these agree. */
  readonly rest: readonly string[];
  /** Of two clashing rows, the one with the larger value here stays; on a tie, the one already under the new key. */
  readonly newer: string;
  /** Runs the copy of the rows of `from` to `to`, keeping what that alone would lose. */
  readonly around?: (database: RerailDatabase, rows: { from: string; to: string }, copy: () => void) => void;
}

const PATH_KEYED_TABLES: readonly PathKeyedTable[] = [
  { table: "checkouts", key: "checkout_path", paths: "1", rest: [], newer: "last_seen_at", around: keepStartState },
  {
    table: "rule_settings",
    key: "owner_id",
    paths: "owner_kind = 'checkout'",
    rest: ["owner_kind", "folder", "target_kind", "target_id"],
    newer: "updated_at",
  },
  { table: "audit_settings", key: "workspace_root", paths: "1", rest: [], newer: "updated_at" },
  { table: "repository_audits", key: "workspace_root", paths: "1", rest: [], newer: "started_at" },
  // A preference keeps no time, and saving one again updates its row in place, so the row created later stays.
  { table: "presentation_preferences", key: "repository", paths: "repository <> ''", rest: ["host"], newer: "rowid" },
];

/**
 * Where an audit ran, which is no key and is never rewritten: each spelling that moved is recorded, and the list of
 * repository audits reads it under its folder's key (`listRepositoryAudits`). That list joins `audits.workspace_root` to
 * `repository_audits` by its text, and an earlier build keyed a Git checkout and its repository audit by Git's spelling
 * while it wrote the audits there under the session's, so a spelling only the history holds is looked up too.
 */
const PATH_HISTORY: readonly PathColumn[] = [
  { table: "audits", key: "workspace_root", paths: "1" },
  { table: "audit_requests", key: "workspace_root", paths: "1" },
];

/** `path_key_spellings` as `path-keys-as-stored` introduced it. */
const PATH_KEY_SPELLINGS_TABLE = "CREATE TABLE IF NOT EXISTS `path_key_spellings` (\n\t`stored` text PRIMARY KEY NOT NULL,\n\t`current` text\n)";

/**
 * The key this build looks a folder up by, for a key an earlier build stored as `stored`. On Windows, for a folder that
 * exists, it is the folder's {@link physicalPath}, which spells each name as the file system stores it: earlier builds
 * kept the spelling a caller passed (`c:\work\app` from VS Code), and the stores of every table above look a folder
 * that exists up by that spelling (`physicalPath`, or `checkoutRealpath` in the audit controls). A folder that is gone
 * keeps `stored`: the file system holds no spelling for it, and the audit controls look it up by the spelling a session
 * there gave, which is the old key. Elsewhere it is `stored` itself, since the realpath there did not change.
 */
export function currentPathKey(stored: string, platform: NodeJS.Platform = process.platform): string {
  return platform === "win32" && existsSync(stored) ? physicalPath(stored, platform) : stored;
}

/**
 * How long the lookups of one drive may take in one open: the keys left once it is spent wait in `path_key_spellings`
 * for a later open (`respellReturnedKeys`). One lookup runs to its end, so a drive can pass its budget by one lookup, and
 * each open that reads a drive looks up at least one of its keys.
 */
const DRIVE_BUDGET_MS = 1_000;

/**
 * The drives and shares an open looks keys up on, each a root as `win32.parse` gives it (`C:\`, `\\host\share\`), and
 * the clock its time per drive runs on.
 */
export interface ReadableRoots {
  readonly roots: readonly string[];
  readonly now: () => number;
}

/**
 * The roots an open reads: this computer's system drive, which holds Windows and is never on a network, and the drive or
 * share of `workspace`, the folder the open's caller works in (a hook's session, a command's checkout), which answered
 * as the caller came to it. An open reads no other root. A drive mapped to a share, or a share, whose host does not
 * answer holds a read about 21 s, and no thread can cut such a read short or end the process before it does: a process
 * that started one waited that long at its exit, however briefly it waited for the answer.
 */
export function readableRoots(workspace: string | undefined, environment: NodeJS.ProcessEnv = process.env): ReadableRoots {
  const system = `${environment.SystemDrive ?? "C:"}\\`;
  return { roots: workspace === undefined ? [system] : [system, win32.parse(workspace).root], now: () => Date.now() };
}

/**
 * `keyOf` on Windows for keys on `readable`'s roots only, within a time budget per root. A key on a drive mapped to a
 * share is looked up through the mapping, which gives the share's spelling, the key this build looks the folder up by. A
 * key on any other drive or share, or past its root's budget, is undefined without a read: it waits for an open whose
 * caller works on its drive (an absent disk, or a share whose host is gone, is then there again).
 */
export function withDriveBudget(keyOf: PathKeyOf, platform: NodeJS.Platform = process.platform, readable = readableRoots(undefined)): PathKeyOf {
  if (platform !== "win32") return keyOf;
  const spent = new Map(readable.roots.map(root => [rootOf(root), 0]));
  return stored => {
    const root = rootOf(stored);
    const used = spent.get(root);
    if (used === undefined || used >= DRIVE_BUDGET_MS) return undefined;
    const started = readable.now();
    try {
      return keyOf(stored);
    } finally {
      spent.set(root, used + readable.now() - started);
    }
  };
}

/** The drive or share `path` lies on, in one letter case: Windows reads `C:\` and `c:\` as one drive. */
function rootOf(path: string): string {
  return win32.parse(path).root.toLowerCase();
}

/**
 * Whether keys wait in `path_key_spellings` (no `current`) on the drive or share of `workspace`: an open made from that
 * folder looks them up (`openRerailDatabaseIn`). None ever waits where keys cannot move (`pathKeysCanMove`), so callers
 * ask only on Windows.
 */
export function keysWaitOn(database: RerailDatabase, workspace: string): boolean {
  if (!holdsTable(database.$client, "path_key_spellings")) return false;
  const root = rootOf(workspace);
  return waitingKeys(database).some(stored => rootOf(stored) === root);
}

/**
 * Before the open takes the write lock: every distinct key the keyed tables and the audit history hold, read in one
 * snapshot, each looked up once (`keyOf`, behind {@link looksUp}). A plain read, which a WAL file runs beside any writer,
 * so neither the history's size, a cold page cache nor a drive's lookups keep another writer waiting.
 */
export function lookUpPathKeys(database: RerailDatabase, keyOf: PathKeyOf): PathKeyLookups {
  const { keys, historyThrough } = readInOneSnapshot(database, () => ({
    keys: storedPathKeys(database, new Map()),
    historyThrough: new Map(PATH_HISTORY.map(column => [column.table, lastRowid(database, column.table)])),
  }));
  const respelled = onceEach(keyOf);
  return { keys: new Map([...keys].map(key => [key, respelled(key)])), historyThrough };
}

/**
 * Before the lock, at each later writable open: each key that waits for its drive (`path_key_spellings` with no
 * `current`), looked up again where `keyOf` reads its drive. One read of a table that is empty at nearly every open; a
 * key on a drive the open does not read costs no lookup, however many opens it waits through.
 */
export function lookUpWaitingKeys(database: RerailDatabase, keyOf: PathKeyOf): PathKeyLookups {
  const waiting = holdsTable(database.$client, "path_key_spellings") ? waitingKeys(database) : [];
  const respelled = onceEach(keyOf);
  return { keys: new Map(waiting.map(stored => [stored, respelled(stored)])), historyThrough: new Map() };
}

/**
 * Under the lock: copies the rows under every path key the file holds to the key `lookups` found for it where that
 * differs, merging the rows that come to one key (the newer stays, `PathKeyedTable.newer`, and a checkout keeps a start
 * state either row had), and records each spelling that moved in `path_key_spellings`, the audit history's among them:
 * those read before the lock, and those of rows written since. A key with no key found, or one written after the
 * lookups (`lookedUp`), goes there with none, for a later open to look up. A key `keyOf` could not resolve (it threw: a
 * folder Rerail may not read) stays as it is, so the open never fails on one folder, and so does a key the step does
 * not look up ({@link looksUp}). `keyOf` must give back unchanged a key it returned, as a realpath does: rows copied to
 * a key that would itself move could stay short of the last one.
 */
export function respellPathKeys(database: RerailDatabase, lookups: PathKeyLookups): void {
  database.run(sql.raw(PATH_KEY_SPELLINGS_TABLE));
  const keys = new Set([...lookups.keys.keys(), ...storedPathKeys(database, lookups.historyThrough)]);
  for (const stored of keys) moveKey(database, stored, lookedUp(lookups, stored));
}

/**
 * Under the lock: copies the rows of each waiting key whose folder `lookups` found back to its key; one that is back and
 * keeps its spelling waits no more. A key that began to wait after the lookups waits for the next open.
 */
export function respellReturnedKeys(database: RerailDatabase, lookups: PathKeyLookups): void {
  if (lookups.keys.size === 0) return;
  for (const stored of waitingKeys(database)) {
    const key = lookups.keys.get(stored);
    if (key === stored) database.run(sql`DELETE FROM path_key_spellings WHERE stored = ${stored}`);
    else if (key !== undefined) moveKey(database, stored, key);
  }
}

function waitingKeys(database: RerailDatabase): string[] {
  return database.all<{ stored: string }>(sql`SELECT stored FROM path_key_spellings WHERE current IS NULL`).map(row => row.stored);
}

/** The key `lookups` found for `stored`; undefined, so it waits for a later open, for a key written since they ran. */
function lookedUp(lookups: PathKeyLookups, stored: string): string | undefined {
  return lookups.keys.has(stored) ? lookups.keys.get(stored) : undefined;
}

/** `read`'s result, every statement of it in one read transaction, so the keys and the history's last rowids agree. */
function readInOneSnapshot<T>(database: RerailDatabase, read: () => T): T {
  database.run(sql.raw("BEGIN"));
  try {
    return read();
  } finally {
    database.run(sql.raw("COMMIT"));
  }
}

function lastRowid(database: RerailDatabase, table: string): number {
  if (!holdsTable(database.$client, table)) return 0;
  return database.get<{ last: number | null }>(sql`SELECT max(rowid) AS last FROM ${sql.raw(table)}`)?.last ?? 0;
}

/**
 * Every distinct path a keyed table holds, and every place the audit history says an audit ran past the rowid
 * `historyThrough` gives its table (all of it for a table it does not list), each once.
 */
function storedPathKeys(database: RerailDatabase, historyThrough: ReadonlyMap<string, number>): Set<string> {
  return new Set([
    ...PATH_KEYED_TABLES.flatMap(column => ifHeld(database, column, () => storedKeys(database, column))),
    ...PATH_HISTORY.flatMap(column => ifHeld(database, column, () => historyKeys(database, column, historyThrough.get(column.table) ?? 0))),
  ]);
}

/** What `read` gives for `column`'s table, or nothing when the file does not hold that table. */
function ifHeld(database: RerailDatabase, column: PathColumn, read: () => string[]): string[] {
  return holdsTable(database.$client, column.table) ? read() : [];
}

/**
 * Records where `stored` stands now: `key` when its folder's key moved, none while its folder is absent. A key that
 * moved has the rows each keyed table holds under it copied there.
 */
function moveKey(database: RerailDatabase, stored: string, key: string | undefined): void {
  if (key === stored) return;
  database.run(sql`INSERT INTO path_key_spellings (stored, current) VALUES (${stored}, ${key ?? null})
    ON CONFLICT (stored) DO UPDATE SET current = excluded.current`);
  if (key !== undefined) copyToKey(database, { from: stored, to: key });
}

function copyToKey(database: RerailDatabase, rows: { from: string; to: string }): void {
  for (const keyed of PATH_KEYED_TABLES) {
    if (holdsTable(database.$client, keyed.table)) copyKeyedRows(database, keyed, rows);
  }
}

/** `keyOf` behind {@link looksUp}, run once per distinct key, each answer kept. */
function onceEach(keyOf: PathKeyOf): PathKeyOf {
  const keys = new Map<string, string | undefined>();
  return stored => {
    if (!keys.has(stored)) keys.set(stored, looksUp(stored) ? keyOrStored(keyOf, stored) : stored);
    return keys.get(stored);
  };
}

/**
 * Whether the step looks `stored` up: only a whole path, as Rerail writes every key. `resolve` changes a relative path,
 * or `/work/app` on Windows, which names no drive. A path on a share (`\\host\share\...`) is one, looked up within its
 * share's time ({@link withDriveBudget}).
 */
function looksUp(stored: string): boolean {
  return resolve(stored) === stored;
}

function keyOrStored(keyOf: PathKeyOf, stored: string): string | undefined {
  try {
    return keyOf(stored);
  } catch {
    return stored;
  }
}

function storedKeys(database: RerailDatabase, column: PathColumn): string[] {
  return database.all<{ key: string }>(
    sql`SELECT DISTINCT ${sql.raw(column.key)} AS key FROM ${sql.raw(column.table)} WHERE ${sql.raw(column.paths)} ORDER BY key`,
  ).map(row => row.key);
}

/**
 * The places the audit history table of `column` says an audit ran, in its rows past rowid `after`: the whole table
 * before the lock, a scan of every row's leaf page, and under it only the rows written since.
 */
function historyKeys(database: RerailDatabase, column: PathColumn, after: number): string[] {
  return database.all<{ key: string }>(
    sql`SELECT DISTINCT ${sql.raw(column.key)} AS key FROM ${sql.raw(column.table)} WHERE rowid > ${after} ORDER BY key`,
  ).map(row => row.key);
}

/** The parts of the statements that copy a table's rows, as SQL. */
function statementParts(keyed: PathKeyedTable) {
  return {
    table: sql.raw(keyed.table),
    key: sql.raw(keyed.key),
    paths: sql.raw(keyed.paths),
    // The rows of another key that hold the same other primary key columns, as `other`.
    same: sql.raw(keyed.rest.map(column => ` AND other.${column} = ${keyed.table}.${column}`).join("")),
  };
}

/** The columns of `keyed`'s table besides its key, as the file has them. */
function copiedColumns(database: RerailDatabase, keyed: PathKeyedTable): string {
  return database.all<{ name: string }>(sql`SELECT name FROM pragma_table_info(${keyed.table})`)
    .map(column => column.name).filter(name => name !== keyed.key).map(name => `"${name}"`).join(", ");
}

/**
 * The rows of `from` copied to `to`, where `to` holds no row as new with the same other key columns; a row of `to` that
 * one of `from` outdates goes first, so no key is held twice. The rows of `from` stay as they are.
 */
function copyKeyedRows(database: RerailDatabase, keyed: PathKeyedTable, rows: { from: string; to: string }): void {
  const copy = (): void => {
    const { table, key, paths, same } = statementParts(keyed);
    const columns = sql.raw(copiedColumns(database, keyed));
    const newer = sql.raw(keyed.newer);
    database.run(sql`DELETE FROM ${table} WHERE ${key} = ${rows.to} AND ${paths} AND EXISTS (SELECT 1 FROM ${table} AS other
      WHERE other.${key} = ${rows.from}${same} AND other.${newer} > ${table}.${newer})`);
    database.run(sql`INSERT INTO ${table} (${key}, ${columns}) SELECT ${rows.to}, ${columns} FROM ${table}
      WHERE ${key} = ${rows.from} AND ${paths} AND NOT EXISTS (SELECT 1 FROM ${table} AS other WHERE other.${key} = ${rows.to}${same})`);
  };
  if (keyed.around === undefined) copy();
  else keyed.around(database, rows, copy);
}

/**
 * Copies a checkout's row keeping its start state: of two rows for one checkout of the same clone, the row that stays
 * under `to` takes the other's start state where it has none, so a checkout started or stopped under one spelling stays
 * so. A row of another clone id (the folder was cloned again) gives none, as recording a re-clone keeps none. The row
 * under `from` keeps its own, for the earlier build that reads it.
 */
function keepStartState(database: RerailDatabase, rows: { from: string; to: string }, copy: () => void): void {
  const before = database.all<{ cloneId: string; started: number | null }>(
    sql`SELECT clone_id AS cloneId, started FROM checkouts WHERE checkout_path IN (${rows.from}, ${rows.to})`,
  );
  copy();
  const after = database.get<{ cloneId: string; started: number | null }>(
    sql`SELECT clone_id AS cloneId, started FROM checkouts WHERE checkout_path = ${rows.to}`,
  );
  if (after === undefined || after.started !== null) return;
  const kept = before.find(row => row.cloneId === after.cloneId && row.started !== null);
  if (kept !== undefined) database.run(sql`UPDATE checkouts SET started = ${kept.started} WHERE checkout_path = ${rows.to}`);
}
