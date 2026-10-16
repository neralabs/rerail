/**
 * The Rerail database's migration path. The Drizzle tables in `schema.ts` are the current shape: a new file
 * gets it from the generated DDL. An existing file reaches it through the steps below, run in list order the
 * next time a writer opens it.
 *
 * `PRAGMA user_version` is the file's compatibility floor: the oldest schema version a build must read to
 * open it. A build refuses a file whose floor is above its own `RERAIL_SCHEMA_VERSION`. A step raises the
 * floor only when an older build would misread the file after it. Older builds read a new nullable column,
 * table or index correctly, because Drizzle names the columns it reads and writes, so such a step sets no
 * `floor` and an older build keeps opening the file. Which steps a file has run is recorded in
 * `schema_migrations`; an id this build does not know (a newer build's step) is left alone.
 */
import { sql } from "drizzle-orm";
import { AUDIT_HISTORY_BACKFILL_STATEMENTS } from "./audit-history.js";
import type { RerailDatabase } from "./database.js";
import { schemaMigrations } from "./schema.js";

/** One change to the shape of an existing table. Ids and order never change once released. */
export interface Migration {
  /** Unique and stable: `schema_migrations` records it once the step has run. */
  readonly id: string;
  /**
   * The schema version a build must read to open the file after this step, for a change an older build
   * would misread (above `RERAIL_BASELINE_SCHEMA_VERSION`). Omitted for a change older builds read correctly.
   */
  readonly floor?: number;
  /**
   * Brings the file from the shape before this step to the shape after it, inside the open's write
   * transaction, where foreign keys are enforced and `PRAGMA foreign_keys` cannot change: a step that rebuilds a
   * table others reference keeps them valid (or sets `PRAGMA defer_foreign_keys`); `rebuildTable` is only for a
   * table nothing references. It must tolerate a table this step changes not existing yet: the generated DDL that
   * runs after the steps creates it in its current shape. `addColumn` and `rebuildTable` do both.
   */
  up(database: RerailDatabase): void;
}

/**
 * The first schema version with a migration path. Files stamped 1 come from releases before it: 0.1.0-next.1 (main
 * before the migration path) wrote the baseline's shape without `schema_migrations`, and such a file is adopted at the
 * baseline (`database.ts`); 0.1.0-next.0 wrote another shape, which is refused, as is any file stamped 2 (development
 * builds only).
 */
export const RERAIL_BASELINE_SCHEMA_VERSION = 3;

/** This build's steps, oldest first. Append only. */
export const RERAIL_MIGRATIONS: readonly Migration[] = [
  {
    // Rule settings gain `suppressions` and `config_json`, and a row may set only those: the
    // `rule_settings_value` CHECK changes, which SQLite can only do by rebuilding the table.
    // No floor, by choice: builds at the baseline keep opening the file, so agent hosts on
    // different installed versions keep sharing the one database. They read only `on` and
    // `mode`: they ignore a row's `suppressions` and `config_json` (the rule runs with its
    // default options and honours suppression comments), and when one resets that rule's or
    // pack's on/off and mode, its `writeSetting` deletes the row and both values with it.
    id: "rule-settings-suppressions-config",
    up(database) {
      rebuildTable(
        database,
        "rule_settings",
        `CREATE TABLE \`rule_settings\` (
	\`owner_kind\` text NOT NULL,
	\`owner_id\` text NOT NULL,
	\`folder\` text DEFAULT '' NOT NULL,
	\`target_kind\` text NOT NULL,
	\`target_id\` text NOT NULL,
	\`on\` integer,
	\`mode\` text,
	\`suppressions\` text,
	\`config_json\` text,
	\`updated_at\` integer NOT NULL,
	PRIMARY KEY(\`owner_kind\`, \`owner_id\`, \`folder\`, \`target_kind\`, \`target_id\`),
	CONSTRAINT "rule_settings_owner_kind" CHECK("rule_settings"."owner_kind" IN ('repository', 'checkout')),
	CONSTRAINT "rule_settings_target_kind" CHECK("rule_settings"."target_kind" IN ('rule', 'pack')),
	CONSTRAINT "rule_settings_mode" CHECK("rule_settings"."mode" IS NULL OR "rule_settings"."mode" IN ('log', 'steer', 'block')),
	CONSTRAINT "rule_settings_suppressions" CHECK("rule_settings"."suppressions" IS NULL OR "rule_settings"."suppressions" IN ('honour', 'ignore')),
	CONSTRAINT "rule_settings_value" CHECK("rule_settings"."on" IS NOT NULL OR "rule_settings"."mode" IS NOT NULL OR "rule_settings"."suppressions" IS NOT NULL OR "rule_settings"."config_json" IS NOT NULL)
)`,
      );
    },
  },
  {
    // The derived audit history the dashboard reads (`audit-history.ts`), filled from the audits a file already
    // holds. Steps run before the DDL, so the tables are written out here as the DDL creates them; the triggers
    // that keep them current come with the DDL right after. No floor: an older build never reads these tables,
    // and its writes to `audits` reach them through the triggers.
    id: "audit-history-derived-tables",
    up(database) {
      if (database.all(sql`SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'audits'`).length === 0) return;
      for (const statement of AUDIT_HISTORY_TABLES) database.run(sql.raw(statement));
      for (const statement of AUDIT_HISTORY_BACKFILL_STATEMENTS) database.run(sql.raw(statement));
    },
  },
];

/** The derived audit history tables as `audit-history-derived-tables` introduced them. */
const AUDIT_HISTORY_TABLES: readonly string[] = [
  "CREATE TABLE IF NOT EXISTS `audit_violations` (\n\t`audit_id` text NOT NULL,\n\t`ordinal` integer NOT NULL,\n\t`session_id` text NOT NULL,\n\t`started_at` integer NOT NULL,\n\t`rule_id` text,\n\t`violation_key` text NOT NULL,\n\t`logged` integer NOT NULL,\n\tPRIMARY KEY(`audit_id`, `ordinal`)\n)",
  "CREATE INDEX IF NOT EXISTS `audit_violations_session_started` ON `audit_violations` (`session_id`,`started_at`)",
  "CREATE INDEX IF NOT EXISTS `audit_violations_started` ON `audit_violations` (`started_at`)",
  "CREATE INDEX IF NOT EXISTS `audit_violations_rule_started` ON `audit_violations` (`rule_id`,`started_at`)",
  "CREATE INDEX IF NOT EXISTS `audit_violations_key` ON `audit_violations` (`session_id`,`rule_id`,`violation_key`,`started_at`)",
  "CREATE INDEX IF NOT EXISTS `audit_violations_key_logged` ON `audit_violations` (`session_id`,`rule_id`,`violation_key`,`started_at`) WHERE \"audit_violations\".\"logged\" = 1",
  "CREATE TABLE IF NOT EXISTS `rule_violation_rollup` (\n\t`session_id` text NOT NULL,\n\t`rule_id` text NOT NULL,\n\t`violation_key` text NOT NULL,\n\t`last_seen_at` integer NOT NULL,\n\t`last_logged_at` integer,\n\tPRIMARY KEY(`session_id`, `rule_id`, `violation_key`)\n)",
  "CREATE INDEX IF NOT EXISTS `rule_violation_rollup_last_seen` ON `rule_violation_rollup` (`last_seen_at`)",
  "CREATE TABLE IF NOT EXISTS `audit_rule_sets` (\n\t`rule_set_id` integer PRIMARY KEY NOT NULL,\n\t`rule_revisions_json` text NOT NULL\n)",
  "CREATE UNIQUE INDEX IF NOT EXISTS `audit_rule_sets_rule_revisions_json_unique` ON `audit_rule_sets` (`rule_revisions_json`)",
  "CREATE TABLE IF NOT EXISTS `audit_rule_set_rules` (\n\t`rule_set_id` integer NOT NULL,\n\t`rule_id` text NOT NULL,\n\tPRIMARY KEY(`rule_set_id`, `rule_id`)\n)",
  "CREATE TABLE IF NOT EXISTS `audit_rule_set_uses` (\n\t`audit_id` text PRIMARY KEY NOT NULL,\n\t`session_id` text NOT NULL,\n\t`started_at` integer NOT NULL,\n\t`rule_set_id` integer NOT NULL\n)",
  "CREATE INDEX IF NOT EXISTS `audit_rule_set_uses_session_started` ON `audit_rule_set_uses` (`session_id`,`started_at`,`rule_set_id`)",
  "CREATE INDEX IF NOT EXISTS `audit_rule_set_uses_started` ON `audit_rule_set_uses` (`started_at`,`rule_set_id`)",
];

/** The schema version `migrations` leave a file at: the baseline, or the highest floor a step raises it to. */
export function schemaVersionOf(migrations: readonly Migration[]): number {
  return Math.max(RERAIL_BASELINE_SCHEMA_VERSION, ...migrations.map(migration => migration.floor ?? RERAIL_BASELINE_SCHEMA_VERSION));
}

/** The schema version this build writes and the newest one it reads. */
export const RERAIL_SCHEMA_VERSION = schemaVersionOf(RERAIL_MIGRATIONS);

const IDENTIFIER = /^[a-z_][a-z0-9_]*$/u;

function identifier(name: string): string {
  if (!IDENTIFIER.test(name)) throw new Error(`Not a schema identifier: ${name}`);
  return `\`${name}\``;
}

/**
 * Adds `column` to `table` with `definition` (`text`, `integer DEFAULT 0 NOT NULL`, ...), unless the table does
 * not exist yet or already has the column. SQLite adds a column in place without copying the table; a
 * `NOT NULL` column needs a `DEFAULT`.
 */
export function addColumn(database: RerailDatabase, table: string, column: string, definition: string): void {
  const columns = database.all<{ name: string }>(sql`SELECT name FROM pragma_table_info(${table})`);
  if (columns.length === 0 || columns.some(existing => existing.name === column)) return;
  database.run(sql.raw(`ALTER TABLE ${identifier(table)} ADD COLUMN ${identifier(column)} ${definition}`));
}

/** The steps of `migrations` an existing file has not recorded, in list order; every step for a file without the record. */
export function pendingMigrations(database: RerailDatabase, migrations: readonly Migration[]): readonly Migration[] {
  const recordsMigrations = database.all(sql`SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'`).length > 0;
  const applied = new Set(
    recordsMigrations ? database.select({ id: schemaMigrations.id }).from(schemaMigrations).all().map(row => row.id) : [],
  );
  return migrations.filter(migration => !applied.has(migration.id));
}

/**
 * Replaces `table` with the one `ddl` creates (a `CREATE TABLE` under the same name, written out in the step so
 * it never follows later schema changes) and copies every row, column by column, unless the table does not exist
 * yet. A column the old table lacks takes its default. For a table with no indexes or triggers of its own and no
 * foreign keys pointing at it: those would not survive the swap.
 */
export function rebuildTable(database: RerailDatabase, table: string, ddl: string): void {
  const columns = database.all<{ name: string }>(sql`SELECT name FROM pragma_table_info(${table})`);
  if (columns.length === 0) return;
  const previous = `${table}_before_rebuild`;
  database.run(sql.raw(`ALTER TABLE ${identifier(table)} RENAME TO ${identifier(previous)}`));
  database.run(sql.raw(ddl));
  const copied = columns.map(column => identifier(column.name)).join(", ");
  database.run(sql.raw(`INSERT INTO ${identifier(table)} (${copied}) SELECT ${copied} FROM ${identifier(previous)}`));
  database.run(sql.raw(`DROP TABLE ${identifier(previous)}`));
}

/**
 * Brings a file a writer holds under `BEGIN IMMEDIATE` to the current shape and returns the floor to stamp.
 * A new file (`fresh`) gets the DDL and every step recorded as run without running it. An existing file
 * runs the steps it has not recorded, in order, then the DDL, which only creates what is missing.
 */
export function migrate(
  database: RerailDatabase,
  { fresh, floor }: { fresh: boolean; floor: number },
  statements: readonly string[],
  migrations: readonly Migration[],
): number {
  const pending = fresh ? [] : pendingMigrations(database, migrations);
  for (const migration of pending) migration.up(database);
  for (const statement of statements) database.run(sql.raw(statement));
  const recorded = fresh ? migrations : pending;
  const appliedAt = Date.now();
  for (const migration of recorded) database.insert(schemaMigrations).values({ id: migration.id, appliedAt }).run();
  return fresh ? schemaVersionOf(migrations) : Math.max(floor, schemaVersionOf(pending));
}
