/**
 * The rows Rerail keys by a folder's path, moved to the key this build gives that folder. Builds before Windows keys
 * went through `realpathSync.native` stored the spelling a caller passed (`c:\work\app` from VS Code, a folder outside
 * Git as it was typed), and this build looks each folder up by the spelling the file system stores.
 * Without the move, a checkout's start state and settings, its audit settings and audit, and its presentation
 * preferences read as unset after the update. The `path-keys-as-stored` step (`migrations.ts`) runs it once per file,
 * inside the open's write transaction.
 */
import { isAbsolute } from "node:path";
import { sql } from "drizzle-orm";
import type { RerailDatabase } from "./database.js";
import { holdsTable } from "./table-probe.js";

/** A table keyed by a path, its names written out here, as a migration step's are, so later schema changes leave it. */
interface PathKeyedTable {
  readonly table: string;
  /** The column that holds the path. */
  readonly key: string;
  /** The rows whose key is a path, as an SQL condition; the table's other rows are left as they are. */
  readonly paths: string;
  /** The other primary key columns: two rows that come to one path clash only when these agree. */
  readonly rest: readonly string[];
  /** Of two clashing rows, the one with the larger value here stays; on a tie, the one already under the new key. */
  readonly newer: string;
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
  { table: "audit_settings", key: "workspace_root", paths: "1", rest: [], newer: "updated_at" },
  { table: "repository_audits", key: "workspace_root", paths: "1", rest: [], newer: "started_at" },
  // No time is kept with a preference: the one saved last under its spelling, the newer row, stays.
  { table: "presentation_preferences", key: "repository", paths: "repository <> ''", rest: ["host"], newer: "rowid" },
];

/**
 * Moves every path key the file holds to `keyOf(key)`, merging the rows that come to one key: the newer row stays
 * (`PathKeyedTable.newer`), and a checkout keeps a start state either of its rows had. A key `keyOf` cannot resolve
 * (it throws: a folder Rerail may not read) or that is not an absolute path stays as it is, so the open never fails on
 * one folder. `keyOf` runs once per distinct key.
 */
export function respellPathKeys(database: RerailDatabase, keyOf: (stored: string) => string): void {
  const respelled = onceEach(keyOf);
  for (const keyed of PATH_KEYED_TABLES) {
    if (!holdsTable(database.$client, keyed.table)) continue;
    for (const stored of storedKeys(database, keyed)) {
      const key = respelled(stored);
      if (key !== stored) moveRows(database, keyed, stored, key);
    }
  }
}

function onceEach(keyOf: (stored: string) => string): (stored: string) => string {
  const keys = new Map<string, string>();
  return stored => {
    let key = keys.get(stored);
    if (key === undefined) {
      key = isAbsolute(stored) ? keyOrStored(keyOf, stored) : stored;
      keys.set(stored, key);
    }
    return key;
  };
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

/** The rows of `from` take the key `to`: of two that then clash, the older goes first, so no key is held twice. */
function moveRows(database: RerailDatabase, keyed: PathKeyedTable, from: string, to: string): void {
  keyed.beforeMove?.(database, from, to);
  const table = sql.raw(keyed.table);
  const key = sql.raw(keyed.key);
  const newer = sql.raw(keyed.newer);
  const paths = sql.raw(keyed.paths);
  const same = sql.raw(keyed.rest.map(column => ` AND other.${column} = ${keyed.table}.${column}`).join(""));
  database.run(sql`DELETE FROM ${table} WHERE ${key} = ${from} AND ${paths} AND EXISTS (SELECT 1 FROM ${table} AS other
    WHERE other.${key} = ${to}${same} AND other.${newer} >= ${table}.${newer})`);
  database.run(sql`DELETE FROM ${table} WHERE ${key} = ${to} AND ${paths} AND EXISTS (SELECT 1 FROM ${table} AS other
    WHERE other.${key} = ${from}${same} AND other.${newer} > ${table}.${newer})`);
  database.run(sql`UPDATE ${table} SET ${key} = ${to} WHERE ${key} = ${from} AND ${paths}`);
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
