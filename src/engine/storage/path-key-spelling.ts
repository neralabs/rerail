/**
 * The rows Rerail keys by a folder's path, copied to the key this build gives that folder. Builds before Windows keys
 * went through `realpathSync.native` stored the spelling a caller passed (`c:\work\app` from VS Code, a folder outside
 * Git as it was typed), and this build looks each folder up by the spelling the file system stores. Without the copy, a
 * checkout's start state and settings, its audit settings and audit, and its presentation preferences read as unset
 * after the update. The rows under the earlier spelling stay as they were, so an earlier build that shares the file
 * (after a downgrade, or a coding agent's hooks from an older install) still reads all of them; `path_key_spellings`
 * records each spelling that moved, which this build's lists leave out (`notAnEarlierSpelling` in `schema.ts`). The
 * `path-keys-as-stored` step (`migrations.ts`) runs it once per file, inside the open's write transaction: it writes a
 * few rows per moved folder and never rewrites the audit history.
 */
import { existsSync, realpathSync } from "node:fs";
import { resolve, win32 } from "node:path";
import { sql } from "drizzle-orm";
import { physicalPath } from "../catalog/checkout-control.js";
import type { RerailDatabase } from "./database.js";
import { holdsTable } from "./table-probe.js";

/**
 * The key this build gives a folder an earlier build keyed as a path, or undefined while the drive that holds it is
 * absent.
 */
export type PathKeyOf = (stored: string) => string | undefined;

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

/** What a drive's root says of the keys on it: looked up, left as they are, or waiting for the drive. */
type DriveAnswer = "local" | "network" | "absent";

/** A drive root that takes this long to answer is taken for a network one, whose every lookup can wait as long. */
const SLOW_DRIVE_MS = 1_000;

/** How {@link onLocalDrives} reads a drive's root: injected by tests. */
export interface DriveProbe {
  readonly realpath: (root: string) => string;
  readonly now: () => number;
}

const THIS_COMPUTER: DriveProbe = { realpath: root => realpathSync.native(root), now: () => Date.now() };

/**
 * `keyOf` on Windows for the keys of a drive whose root resolves to a local volume, each drive's root read once. A key
 * on a drive whose root resolves to a network share (a mapped drive), or answers only after {@link SLOW_DRIVE_MS},
 * stays as it is, as a key on a share does ({@link looksUp}): a lookup there waits on the network, about 21 s for a
 * host that does not answer, while the open holds the database's write lock. A key on a drive that is absent (a disk not
 * attached, a locked volume) is undefined, to be looked up again at a later open: its folder can come back as it was.
 */
export function onLocalDrives(keyOf: PathKeyOf, platform: NodeJS.Platform = process.platform, probe = THIS_COMPUTER): PathKeyOf {
  if (platform !== "win32") return keyOf;
  const drives = new Map<string, DriveAnswer>();
  return stored => {
    const root = win32.parse(stored).root;
    let drive = drives.get(root);
    if (drive === undefined) {
      drive = driveAnswer(root, probe);
      drives.set(root, drive);
    }
    if (drive === "network") return stored;
    return drive === "absent" ? undefined : keyOf(stored);
  };
}

function driveAnswer(root: string, probe: DriveProbe): DriveAnswer {
  const started = probe.now();
  let resolved: string | undefined;
  try {
    resolved = probe.realpath(root);
  } catch {
    resolved = undefined;
  }
  if (probe.now() - started >= SLOW_DRIVE_MS || resolved?.startsWith("\\\\") === true) return "network";
  return resolved === undefined ? "absent" : "local";
}

/**
 * Copies the rows under every path key the file holds to `keyOf(key)` where that differs, merging the rows that come to
 * one key (the newer stays, `PathKeyedTable.newer`, and a checkout keeps a start state either row had), and records each
 * spelling that moved in `path_key_spellings`, where also goes, with no key, each key on a drive that is absent. A key
 * `keyOf` cannot resolve (it throws: a folder Rerail may not read) stays as it is, so the open never fails on one folder,
 * and so does a key the step does not look up ({@link looksUp}). `keyOf` runs once per distinct key, the audit
 * history's included, and must give back unchanged a key it returned, as a realpath does: rows copied to a key that would
 * itself move could stay short of the last one.
 */
export function respellPathKeys(database: RerailDatabase, keyOf: PathKeyOf): void {
  database.run(sql.raw(PATH_KEY_SPELLINGS_TABLE));
  const respelled = onceEach(keyOf);
  for (const stored of storedPathKeys(database)) moveKey(database, stored, respelled(stored));
}

/**
 * Looks again at the keys whose drive was absent when they were looked up, and copies the rows of each folder that is
 * back to its key; a key whose drive is back and that keeps its spelling waits no more. One read of a table that is
 * empty at nearly every open.
 */
export function respellReturnedKeys(database: RerailDatabase, keyOf: PathKeyOf): void {
  const waiting = database.all<{ stored: string }>(sql`SELECT stored FROM path_key_spellings WHERE current IS NULL`);
  if (waiting.length === 0) return;
  const respelled = onceEach(keyOf);
  for (const { stored } of waiting) {
    const key = respelled(stored);
    if (key === stored) database.run(sql`DELETE FROM path_key_spellings WHERE stored = ${stored}`);
    else if (key !== undefined) moveKey(database, stored, key);
  }
}

/** Every distinct path a keyed table or the audit history holds, each once. */
function storedPathKeys(database: RerailDatabase): Set<string> {
  const keys = new Set<string>();
  for (const column of [...PATH_KEYED_TABLES, ...PATH_HISTORY]) {
    if (holdsTable(database.$client, column.table)) for (const key of storedKeys(database, column)) keys.add(key);
  }
  return keys;
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
 * Whether the step looks `stored` up. Only a whole path, as Rerail writes every key: `resolve` changes a relative path,
 * or `/work/app` on Windows, which names no drive. Never a path on a network share (`\\host\share\...`): a lookup there
 * waits on the network, while the open holds the database's write lock.
 */
function looksUp(stored: string): boolean {
  return resolve(stored) === stored && !stored.startsWith("\\\\");
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
