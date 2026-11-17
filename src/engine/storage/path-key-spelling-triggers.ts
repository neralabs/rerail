/**
 * Carries each write an earlier build makes under a folder spelling `path_key_spellings` records over to the key this
 * build gives that folder. The `path-keys-as-stored` step (`path-key-spelling.ts`) copies a folder's rows once and keeps
 * the earlier build's rows; an earlier build that shares the file (after a downgrade, or a coding agent's hooks from an
 * older install) goes on writing under its own spelling. Without these triggers this build would go on reading the copy
 * as the step made it: a stop made there, a rule turned back on, Log mode set back to Steer, a status line set back to
 * compact, or an error `rerail scrub` cleaned would not reach it. They are triggers because only a trigger runs inside an
 * earlier build's own writes.
 *
 * What carries over: a checkout's start state when it changes; and for audit settings, a checkout's rule settings, its
 * repository audit and its presentation preferences, each row an earlier build inserts or changes (unless this build's
 * row for the folder is newer, `CarriedTable.newer`) and each row it deletes. A write under any other key finds no
 * spelling and changes nothing, so this build's own writes cost one lookup of that table's primary key. On Windows an
 * earlier build's start or stop of a folder also sets this build's row of it, in another letter case, to follow its
 * own, so a start state reads that build's row as a containing checkout (`startStateParents`). Nothing else goes the
 * other way: an earlier build reads its own rows, as beside a build without the step.
 *
 * Like the audit history triggers (`audit-history.ts`) they are part of the DDL applied on every writable open, created
 * `IF NOT EXISTS`: a build that changes one must drop it in a migration step so the new body is created. They use no
 * conflict clause, which an outer `INSERT OR REPLACE` would override, and set only values the earlier build's own row
 * holds, which passed the same constraints, so they never fail the write that runs them.
 */

/** A table keyed by a folder's path whose rows an earlier build's writes carry over. */
interface CarriedTable {
  readonly table: string;
  /** The column that holds the path. */
  readonly key: string;
  /** The other primary key columns. */
  readonly rest: readonly string[];
  /** The columns a write carries over. */
  readonly values: readonly string[];
  /** The columns whose change is the earlier build's change; a write that leaves them as they were carries nothing. */
  readonly changes: readonly string[];
  /** Of this build's row and the earlier build's, this build's stays where its value here is larger. */
  readonly newer?: string;
  /** The rows whose key is a path, as a condition on the row a trigger holds (`NEW`, `OLD`); the others stay as they are. */
  readonly paths?: (row: string) => string;
}

const CARRIED_TABLES: readonly CarriedTable[] = [
  {
    table: "rule_settings",
    key: "owner_id",
    rest: ["owner_kind", "folder", "target_kind", "target_id"],
    values: ["\"on\"", "mode", "suppressions", "config_json", "updated_at"],
    changes: ["\"on\"", "mode", "suppressions", "config_json"],
    newer: "updated_at",
    paths: row => `${row}.owner_kind = 'checkout'`,
  },
  { table: "audit_settings", key: "workspace_root", rest: [], values: ["settings_json", "updated_at"], changes: ["settings_json"], newer: "updated_at" },
  {
    table: "repository_audits",
    key: "workspace_root",
    rest: [],
    values: ["session_id", "audit_id", "status", "started_at", "finished_at", "error"],
    changes: ["session_id", "audit_id", "status", "started_at", "finished_at", "error"],
    newer: "started_at",
  },
  {
    table: "presentation_preferences",
    key: "repository",
    rest: ["host"],
    values: ["presence"],
    changes: ["presence"],
    paths: row => `${row}.repository <> ''`,
  },
];

/** The key this build gives the folder `row`'s key names, or NULL where that key is no earlier spelling. */
const currentKey = (carried: CarriedTable, row: string): string =>
  `(SELECT current FROM path_key_spellings WHERE stored = ${row}.${carried.key})`;

/** The rest of the primary key equal to `row`'s, as conditions that follow another. */
const sameRest = (carried: CarriedTable, row: string): string => carried.rest.map(column => ` AND ${column} = ${row}.${column}`).join("");

/** `conditions` joined as a trigger's `WHEN`, or no `WHEN` at all. */
const when = (conditions: ReadonlyArray<string | undefined>): string => {
  const held = conditions.filter(condition => condition !== undefined);
  return held.length === 0 ? "" : `\n  WHEN ${held.join(" AND ")}`;
};

/**
 * The earlier build's row (`NEW`) made under this build's key: this build's row takes its values unless that row is
 * newer, and where there is none, the row is copied.
 */
function carryRow(carried: CarriedTable): string {
  const { table, key } = carried;
  const columns = [...carried.rest, ...carried.values];
  const newer = carried.newer === undefined ? "" : ` AND ${carried.newer} <= NEW.${carried.newer}`;
  return `
  UPDATE ${table} SET ${carried.values.map(column => `${column} = NEW.${column}`).join(", ")}
    WHERE ${key} = ${currentKey(carried, "NEW")}${sameRest(carried, "NEW")}${newer};
  INSERT INTO ${table} (${key}, ${columns.join(", ")})
    SELECT spelling.current, ${columns.map(column => `NEW.${column}`).join(", ")} FROM path_key_spellings AS spelling
    WHERE spelling.stored = NEW.${key} AND spelling.current IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM ${table} WHERE ${key} = spelling.current${sameRest(carried, "NEW")});`;
}

/** The insert, update and delete triggers of one carried table. */
function carriedTableTriggers(carried: CarriedTable): string[] {
  const { table } = carried;
  const changed = `(${carried.changes.map(column => `NEW.${column} IS NOT OLD.${column}`).join(" OR ")})`;
  return [
    `CREATE TRIGGER IF NOT EXISTS \`${table}_earlier_spelling_insert\` AFTER INSERT ON \`${table}\`${when([carried.paths?.("NEW")])}
  BEGIN${carryRow(carried)}
  END`,
    `CREATE TRIGGER IF NOT EXISTS \`${table}_earlier_spelling_update\` AFTER UPDATE ON \`${table}\`${when([carried.paths?.("NEW"), changed])}
  BEGIN${carryRow(carried)}
  END`,
    `CREATE TRIGGER IF NOT EXISTS \`${table}_earlier_spelling_delete\` AFTER DELETE ON \`${table}\`${when([carried.paths?.("OLD")])}
  BEGIN
  DELETE FROM ${table} WHERE ${carried.key} = ${currentKey(carried, "OLD")}${sameRest(carried, "OLD")};
  END`,
  ];
}

/**
 * Applied, in order, after the tables on every open; see the module comment before changing one. A checkout carries
 * only its start state, for the same clone: an earlier build touches its row at every session (`last_seen_at`), and a
 * re-clone it records is recorded again by this build at its own next session there.
 */
export const PATH_KEY_SPELLING_TRIGGER_STATEMENTS: readonly string[] = [
  `CREATE TRIGGER IF NOT EXISTS \`checkouts_earlier_spelling_started\` AFTER UPDATE OF started ON \`checkouts\`
  WHEN NEW.started IS NOT OLD.started
  BEGIN
  UPDATE checkouts SET started = NEW.started
    WHERE checkout_path = (SELECT current FROM path_key_spellings WHERE stored = NEW.checkout_path) AND clone_id = NEW.clone_id;
  END`,
  ...CARRIED_TABLES.flatMap(carriedTableTriggers),
];
