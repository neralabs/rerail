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
  // A preference keeps no time, and saving one again updates its row in place, so the row created later stays.
  { table: "presentation_preferences", key: "repository", paths: "repository <> ''", rest: ["host"], newer: "rowid" },
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
 * itself move could stay short of the last one.
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
