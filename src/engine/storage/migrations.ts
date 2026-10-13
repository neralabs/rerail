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
   * table others reference keeps them valid (or sets `PRAGMA defer_foreign_keys`). It must tolerate a table this
   * step changes not existing yet: the generated DDL that runs after the steps creates it in its current shape.
   * `addColumn` does both.
   */
  up(database: RerailDatabase): void;
}

/**
 * The first schema version with a migration path. Files stamped below it (1, or 2 from development builds)
 * predate it, carry no `schema_migrations`, and are refused: there is no release whose files they could be.
 */
export const RERAIL_BASELINE_SCHEMA_VERSION = 3;

/** This build's steps, oldest first. Append only. */
export const RERAIL_MIGRATIONS: readonly Migration[] = [];

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
