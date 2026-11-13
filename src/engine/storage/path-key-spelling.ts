/**
 * The rows Rerail keys by a folder's path, moved to the key this build gives that folder. Builds before Windows keys
 * went through `realpathSync.native` stored the spelling a caller passed (`c:\work\app` from VS Code, a folder outside
 * Git as it was typed), and this build looks each folder up by the spelling the file system stores.
 * Without the move, a checkout's start state and settings, its audit settings and audit, and its presentation
 * preferences read as unset after the update. The `path-keys-as-stored` step (`migrations.ts`) runs it once per file,
 * inside the open's write transaction.
 */
import { existsSync } from "node:fs";
import { resolve } from "node:path";
import { sql } from "drizzle-orm";
import { physicalPath } from "../catalog/checkout-control.js";
import type { RerailDatabase } from "./database.js";
import { holdsTable } from "./table-probe.js";

/** A column that holds a path, its names written out here, as a migration step's are, so later schema changes leave it. */
interface PathColumn {
  readonly table: string;
  /** The column that holds the path. */
  readonly key: string;
}

/** A table keyed by a path. */
interface PathKeyedTable extends PathColumn {
  /** The rows whose key is a path, as an SQL condition; the table's other rows are left as they are. */
  readonly paths: string;
  /** The other primary key columns: two rows that come to one path clash only when these agree. */
  readonly rest: readonly string[];
  /** Of two clashing rows, the one with the larger value here stays; on a tie, the one already under the new key. */
  readonly newer: string;
  /**
   * The table's columns past the key, given for a table no reader lists: its rows are copied to the new key, and the
   * rows under the old key stay, so an earlier build that opens the file after this one still reads them.
   */
  readonly copied?: readonly string[];
  /** Run before the rows of `from` move to `to`. */
  readonly beforeMove?: (database: RerailDatabase, from: string, to: string) => void;
}

const PATH_KEYED_TABLES: readonly PathKeyedTable[] = [
  { table: "checkouts", key: "checkout_path", paths: "1", rest: [], newer: "last_seen_at", beforeMove: keepStartState },
  {
    table: "rule_settings",
    key: "owner_id",
    paths: "owner_kind = 'checkout'",
    rest: ["owner_kind", "folder", "target_kind", "target_id"],
    newer: "updated_at",
  },
  // Read only by its key (the audit controls), so a copy under the old key is never listed beside the new one.
  {
    table: "audit_settings",
    key: "workspace_root",
    paths: "1",
    rest: [],
    newer: "updated_at",
    copied: ["settings_json", "updated_at"],
  },
  { table: "repository_audits", key: "workspace_root", paths: "1", rest: [], newer: "started_at" },
  // A preference keeps no time, and saving one again updates its row in place, so the row created later stays.
  { table: "presentation_preferences", key: "repository", paths: "repository <> ''", rest: ["host"], newer: "rowid" },
];

/**
 * Where an audit ran, which is no key: each value moves to the key this build gives its folder, with no merge, whether
 * or not a key above held it. The list of repository audits joins `audits.workspace_root` to `repository_audits` by its
 * text, a finished audit request writes its `workspace_root` as a `repository_audits` key, and an earlier build keyed a
 * Git checkout and its repository audit by Git's spelling while it wrote the audits there under the session's.
 */
const PATH_HISTORY: readonly PathColumn[] = [
  { table: "audits", key: "workspace_root" },
  { table: "audit_requests", key: "workspace_root" },
];

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
 * Moves every path key the file holds to `keyOf(key)`, merging the rows that come to one key: the newer row stays
 * (`PathKeyedTable.newer`), and a checkout keeps a start state either of its rows had. A key `keyOf` cannot resolve
 * (it throws: a folder Rerail may not read) stays as it is, so the open never fails on one folder, and so does a key
 * the step does not look up ({@link looksUp}). `keyOf` runs once per distinct key, and must give back unchanged a key
 * it returned, as a realpath does: each table's keys are read once and moved in turn, so rows moved to a key that would
 * itself move could stay short of the last one. Then where an audit ran moves the same way ({@link PATH_HISTORY}),
 * each distinct value looked up as a key is.
 */
export function respellPathKeys(database: RerailDatabase, keyOf: (stored: string) => string): void {
  const keys = new Map<string, string>();
  const respelled = onceEach(keyOf, keys);
  for (const keyed of PATH_KEYED_TABLES) {
    if (holdsTable(database.$client, keyed.table)) respellRows(database, keyed, respelled);
  }
  lookUpHistory(database, respelled);
  respellHistory(database, new Map([...keys].filter(([stored, key]) => key !== stored)));
}

/**
 * Looks each distinct value of a {@link PATH_HISTORY} column up through `respelled`, so a spelling that only the audit
 * history holds moves too. One read of each table, which has no index on the column, once per file.
 */
function lookUpHistory(database: RerailDatabase, respelled: (stored: string) => string): void {
  for (const history of PATH_HISTORY) {
    if (!holdsTable(database.$client, history.table)) continue;
    const values = database.all<{ value: string }>(sql`SELECT DISTINCT ${sql.raw(history.key)} AS value FROM ${sql.raw(history.table)}`);
    for (const { value } of values) respelled(value);
  }
}

function respellRows(database: RerailDatabase, keyed: PathKeyedTable, respelled: (stored: string) => string): void {
  for (const stored of storedKeys(database, keyed)) {
    const key = respelled(stored);
    if (key !== stored) moveRows(database, keyed, stored, key);
  }
}

/**
 * Each value of a {@link PATH_HISTORY} column that is a key `moved` names takes the key it moved to, in one statement
 * per table: the columns have no index, so each statement reads its whole table, and only when a key moved.
 */
function respellHistory(database: RerailDatabase, moved: ReadonlyMap<string, string>): void {
  if (moved.size === 0) return;
  const pairs = JSON.stringify(Object.fromEntries(moved));
  for (const history of PATH_HISTORY) {
    if (!holdsTable(database.$client, history.table)) continue;
    const table = sql.raw(history.table);
    const key = sql.raw(history.key);
    database.run(sql`UPDATE ${table} SET ${key} = (SELECT moves.value FROM json_each(${pairs}) AS moves
      WHERE moves.key = ${table}.${key}) WHERE ${key} IN (SELECT moves.key FROM json_each(${pairs}) AS moves)`);
  }
}

/** `keyOf` behind {@link looksUp}, run once per distinct key, each answer kept in `keys`. */
function onceEach(keyOf: (stored: string) => string, keys: Map<string, string>): (stored: string) => string {
  return stored => {
    let key = keys.get(stored);
    if (key === undefined) {
      key = looksUp(stored) ? keyOrStored(keyOf, stored) : stored;
      keys.set(stored, key);
    }
    return key;
  };
}

/**
 * Whether the step looks `stored` up. Only a whole path, as Rerail writes every key: `resolve` changes a relative path,
 * or `/work/app` on Windows, which names no drive. Never a path on a network share (`\\host\share\...`): a lookup there
 * waits on the network, about 21 s for a host that does not answer, while the open holds the database's write lock.
 */
function looksUp(stored: string): boolean {
  return resolve(stored) === stored && !stored.startsWith("\\\\");
}

function keyOrStored(keyOf: (stored: string) => string, stored: string): string {
  try {
    return keyOf(stored);
  } catch {
    return stored;
  }
}

function storedKeys(database: RerailDatabase, keyed: PathKeyedTable): string[] {
  return database.all<{ key: string }>(
    sql`SELECT DISTINCT ${sql.raw(keyed.key)} AS key FROM ${sql.raw(keyed.table)} WHERE ${sql.raw(keyed.paths)} ORDER BY key`,
  ).map(row => row.key);
}

/**
 * The rows of `from` come to the key `to`: renamed, or copied where the table names `copied` columns. Of two rows that
 * then clash, the older goes first, so no key is held twice.
 */
function moveRows(database: RerailDatabase, keyed: PathKeyedTable, from: string, to: string): void {
  keyed.beforeMove?.(database, from, to);
  if (keyed.copied === undefined) renameRows(database, keyed, from, to);
  else copyRows(database, keyed, { from, to, copied: keyed.copied });
}

/** The parts of the statements that move a table's rows, as SQL. */
function statementParts(keyed: PathKeyedTable) {
  return {
    table: sql.raw(keyed.table),
    key: sql.raw(keyed.key),
    paths: sql.raw(keyed.paths),
    // The rows of another key that hold the same other primary key columns, as `other`.
    same: sql.raw(keyed.rest.map(column => ` AND other.${column} = ${keyed.table}.${column}`).join("")),
  };
}

/** The rows of `drop` that a row of `by` outdates: a newer one, or as new when `ties`, with the same `rest`. */
function dropOutdated(database: RerailDatabase, keyed: PathKeyedTable, rows: { drop: string; by: string; ties: boolean }): void {
  const { table, key, paths, same } = statementParts(keyed);
  const newer = sql.raw(keyed.newer);
  const than = sql.raw(rows.ties ? ">=" : ">");
  database.run(sql`DELETE FROM ${table} WHERE ${key} = ${rows.drop} AND ${paths} AND EXISTS (SELECT 1 FROM ${table} AS other
    WHERE other.${key} = ${rows.by}${same} AND other.${newer} ${than} ${table}.${newer})`);
}

function renameRows(database: RerailDatabase, keyed: PathKeyedTable, from: string, to: string): void {
  const { table, key, paths } = statementParts(keyed);
  dropOutdated(database, keyed, { drop: from, by: to, ties: true });
  dropOutdated(database, keyed, { drop: to, by: from, ties: false });
  database.run(sql`UPDATE ${table} SET ${key} = ${to} WHERE ${key} = ${from} AND ${paths}`);
}

/** The rows of `from` copied to `to` where `to` holds no row as new; the rows of `from` stay as they are. */
function copyRows(database: RerailDatabase, keyed: PathKeyedTable, rows: { from: string; to: string; copied: readonly string[] }): void {
  const { table, key, paths, same } = statementParts(keyed);
  const columns = sql.raw(rows.copied.join(", "));
  dropOutdated(database, keyed, { drop: rows.to, by: rows.from, ties: false });
  database.run(sql`INSERT INTO ${table} (${key}, ${columns}) SELECT ${rows.to}, ${columns} FROM ${table}
    WHERE ${key} = ${rows.from} AND ${paths} AND NOT EXISTS (SELECT 1 FROM ${table} AS other WHERE other.${key} = ${rows.to}${same})`);
}

/**
 * Of two rows for one checkout of the same clone, each takes the other's start state where it has none of its own, so
 * a checkout started or stopped under one spelling stays so whichever row stays. A row of another clone id (the folder
 * was cloned again) gives none, as recording a re-clone keeps none.
 */
function keepStartState(database: RerailDatabase, from: string, to: string): void {
  for (const [row, other] of [[from, to], [to, from]] as const) {
    database.run(sql`UPDATE checkouts SET started = (SELECT other.started FROM checkouts AS other
      WHERE other.checkout_path = ${other} AND other.clone_id = checkouts.clone_id)
      WHERE checkout_path = ${row} AND started IS NULL`);
  }
}
