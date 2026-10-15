import { accessSync, chmodSync, constants, existsSync, mkdirSync, readFileSync } from "node:fs";
import Database from "better-sqlite3";
import { dirname, join } from "node:path";
import { sql } from "drizzle-orm";
import { drizzle, type BetterSQLite3Database } from "drizzle-orm/better-sqlite3";
import * as schema from "./schema.js";
import { RERAIL_BASELINE_SCHEMA_VERSION, RERAIL_MIGRATIONS, migrate, pendingMigrations, schemaVersionOf, type Migration } from "./migrations.js";
import { RERAIL_BASELINE_SCHEMA_STATEMENTS } from "./baseline-schema.js";
import { RERAIL_SCHEMA_STATEMENTS } from "./schema-sql.generated.js";
import { enableWal } from "./sqlite-wal.js";
import { defaultRerailHome } from "./rerail-home.js";

export type RerailDatabase = BetterSQLite3Database<typeof schema> & { readonly $client: Database.Database };

export interface OpenRerailDatabaseOptions {
  /** Open an existing database without write access; the schema version is checked but the schema is not applied. */
  readonly?: boolean;
  /** The migration steps a writable open runs: this build's (`RERAIL_MIGRATIONS`) unless a test passes its own. */
  migrations?: readonly Migration[];
}

/** The one Rerail database: rules and every repository's history, in the Rerail home. */
export const RERAIL_DATABASE_FILE = "rerail.sqlite";

/** `$RERAIL_HOME/rerail.sqlite`, with the Rerail home defaulting to `~/.rerail`. */
export function defaultRerailDatabasePath(environment: NodeJS.ProcessEnv = process.env): string {
  return join(defaultRerailHome(environment), RERAIL_DATABASE_FILE);
}

/**
 * Beside the database while `rerail update rollback` writes a copy into it (`project-cli.ts`), holding that process's
 * pid. Every opener refuses the database while it is live (`rollbackRestoring`).
 */
export function rollbackRestoringPath(path: string): string {
  return `${path}.restoring`;
}

/**
 * Whether a rollback is writing a copy into the database at `path`: its `.restoring` file exists and the process it
 * names still runs. A file whose process is gone is left by a rollback that was killed, and holds nothing back; one
 * that names no process (unreadable, or written by an earlier build) is taken as live.
 */
export function rollbackRestoring(path: string): boolean {
  let text: string;
  try {
    text = readFileSync(rollbackRestoringPath(path), "utf8");
  } catch (error) {
    return (error as NodeJS.ErrnoException).code !== "ENOENT";
  }
  const pid = Number.parseInt(text.trim(), 10);
  if (!Number.isSafeInteger(pid) || pid <= 0) return true;
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return (error as NodeJS.ErrnoException).code !== "ESRCH";
  }
}

/**
 * Refuses a connection while a rollback writes a copy into the database; checked after a read, so the connection
 * already holds what makes the rollback's journal switch fail (WAL keeps a reader's shared lock until it closes). A
 * connection that read before the rollback's file existed makes the rollback refuse; one that reads after sees it.
 */
function refuseWhileRestoring(client: Database.Database, path: string): void {
  client.pragma("user_version", { simple: true });
  if (rollbackRestoring(path)) {
    throw new Error(
      `Rerail is putting back a copy of ${path} for a rollback; try again in a moment (if no rerail update rollback is `
        + `running, one was interrupted: delete ${rollbackRestoringPath(path)})`,
    );
  }
}

/**
 * A connection that has changed nothing on disk yet; a writable one creates the directory and an absent file. A
 * writable open of a file this process cannot write throws before anything else touches it: SQLite would quietly open
 * it read-only, and the writer would then change its mode before failing. No connection opens while a rollback writes
 * a copy into the file (`refuseWhileRestoring`).
 */
function connectSqlite(path: string, readonly: boolean): RerailDatabase {
  if (!readonly) {
    mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
    // The folder too: SQLite writes its journal and WAL beside the file.
    for (const target of existsSync(path) ? [path, dirname(path)] : []) accessSync(target, constants.W_OK);
  }
  const client = new Database(path, { readonly });
  try {
    const database = drizzle(client, { schema });
    database.run(sql`PRAGMA busy_timeout = 5000`);
    refuseWhileRestoring(client, path);
    return database;
  } catch (error) {
    client.close();
    throw error;
  }
}

/**
 * What a writer owns: the file mode, WAL, and the connection's integrity settings. The rollback check runs again in
 * WAL mode: a file a rollback left in rollback-journal mode holds no lock for a connection between its reads.
 */
function prepareWriter(database: RerailDatabase, path: string): void {
  chmodSync(path, 0o600);
  enableWal(() => database.run(sql`PRAGMA journal_mode = WAL`));
  refuseWhileRestoring(database.$client, path);
  database.run(sql`PRAGMA foreign_keys = ON`);
  database.run(sql`PRAGMA synchronous = FULL`);
}

/** Open the Node-owned SQLite database without applying Rerail's project schema. */
export function openSqliteDatabase(path: string, options: OpenRerailDatabaseOptions = {}): RerailDatabase {
  const readonly = options.readonly === true;
  const database = connectSqlite(path, readonly);
  try {
    if (!readonly) prepareWriter(database, path);
    return database;
  } catch (error) {
    database.$client.close();
    throw error;
  }
}

/** What a file on disk records about its schema. */
interface SchemaStamp {
  /** Whether the file holds any table; a file without one is new while it carries no stamp either. */
  readonly empty: boolean;
  /** `PRAGMA user_version`: the file's compatibility floor (`migrations.ts`), 0 when no Rerail stamped it. */
  readonly version: number;
  /** For a file stamped 1: whether it holds exactly the baseline's shape without `schema_migrations` (0.1.0-next.1). */
  readonly adoptable: boolean;
}

/** Every schema entry of `client`'s database as SQLite records it (type, name, table, CREATE statement), in order. */
function schemaEntries(client: Database.Database): string[] {
  return client.prepare("SELECT type || ' ' || name || ' ' || tbl_name || ' ' || ifnull(sql, '') FROM sqlite_master "
    + "WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name").pluck().all() as string[];
}

let baselineWithoutRecord: readonly string[] | undefined;

/** The baseline's schema entries without `schema_migrations`: the shape 0.1.0-next.1 (main before the path) wrote. */
function stampOneShape(): readonly string[] {
  if (baselineWithoutRecord !== undefined) return baselineWithoutRecord;
  const memory = new Database(":memory:");
  try {
    for (const statement of RERAIL_BASELINE_SCHEMA_STATEMENTS) memory.exec(statement);
    baselineWithoutRecord = schemaEntries(memory).filter(entry => !/^\S+ \S+ schema_migrations /u.test(entry));
  } finally {
    memory.close();
  }
  return baselineWithoutRecord;
}

/**
 * The file's stamp, read as one snapshot: another opener adopting a file stamped 1 between reading its stamp and
 * reading its schema would otherwise make an adoptable file look like one in an unknown shape.
 */
function schemaStamp(database: RerailDatabase): SchemaStamp {
  return database.$client.transaction(() => {
    const tables = database.all<{ name: string }>(sql`SELECT name FROM sqlite_master WHERE type = 'table'`);
    const version = database.all<{ user_version: number }>(sql`PRAGMA user_version`)[0]?.user_version ?? 0;
    const adoptable = version === 1 && JSON.stringify(schemaEntries(database.$client)) === JSON.stringify(stampOneShape());
    return { empty: tables.length === 0, version, adoptable };
  })();
}

/** The refusal for a file that holds something other than a Rerail database this build can read. */
function unreadableDatabase(path: string, recorded: string, supported: number): string {
  return `${path} is not a database this build can open (${recorded}; this build reads schema v${supported}). `
    + "Move the file aside to start fresh.";
}

/** SQLite's "file is not a database", raised directly or as the cause of Drizzle's query error. */
function notADatabase(error: unknown): boolean {
  const notSqlite = (value: unknown): boolean => value instanceof Database.SqliteError && value.code === "SQLITE_NOTADB";
  return notSqlite(error) || (error instanceof Error && notSqlite(error.cause));
}

/** An error that says this process may not write the file or its folder. */
function cannotWrite(error: unknown): boolean {
  const code = (error as { code?: unknown } | undefined)?.code;
  return typeof code === "string" && (["EACCES", "EPERM", "EROFS"].includes(code) || code.startsWith("SQLITE_READONLY"));
}

/**
 * Why a build reading schemas up to `supported` cannot open a file stamped `stamp`, or undefined when it can: a
 * file stamped by a newer Rerail, a file that is not new and carries no Rerail stamp (a `user_version` below 1),
 * or a file from before the migration baseline other than one in 0.1.0-next.1's shape, which a writer adopts. Only
 * a file with neither a table nor a stamp is new; a stamped file is judged by its stamp even when it holds no table.
 */
function schemaRefusal(path: string, stamp: SchemaStamp, supported: number): string | undefined {
  if (stamp.version > supported) {
    return `${path} was written by a newer Rerail (schema v${stamp.version}); this Rerail supports schema v${supported}. `
      + "Update Rerail to open it, or move the file aside to start fresh.";
  }
  if (stamp.version === 0 && stamp.empty) return undefined;
  if (stamp.version < 1) return unreadableDatabase(path, "no Rerail schema version", supported);
  if (stamp.version === 1 && !stamp.adoptable) {
    return unreadableDatabase(path, "schema v1 in a shape this build does not know, such as 0.1.0-next.0's", supported);
  }
  if (stamp.version === 2) {
    return unreadableDatabase(path, "schema v2, from a development build before the migration baseline", supported);
  }
  return undefined;
}

/**
 * Open the Rerail database, creating the directory, file, and schema as needed.
 * Writers own the file mode (0600) and directory mode (0700) they create.
 *
 * `PRAGMA user_version` holds the file's compatibility floor. A file stamped by a newer Rerail, a file with
 * tables or a stamp no Rerail wrote, a file from before the migration baseline, and a file that is not SQLite at
 * all are refused, writable or read-only, and left untouched: a writable open checks the stamp before it sets
 * the file mode or switches the journal to WAL. Otherwise a writable open runs the migration steps the file has
 * not run and the current DDL, which is idempotent, and stamps the floor they reach (`migrations.ts`).
 *
 * A read-only open never writes its own connection. Its readers query this build's shape, so when an existing file
 * is in 0.1.0-next.1's shape (stamped 1), or still lacks steps that keep the floor (columns and tables older builds
 * read correctly, the upgrade a new version makes when it first runs), a writable open adopts it or runs them first.
 * A reader that cannot write the file (permissions) is refused with the reason, since this build's readers would fail
 * on the shape it has; any other failure of that open is the open's own. A step that raises the floor waits for this
 * version's writer, since an older Rerail still running would misread the file after it, so a read-only open of a
 * file with one pending is refused, telling the person to run a Rerail command first. A read-only open of a file
 * whose creator has not committed the schema yet sees no table at all.
 */
export function openRerailDatabase(path: string, options: OpenRerailDatabaseOptions = {}): RerailDatabase {
  const readonly = options.readonly === true;
  const migrations = options.migrations ?? RERAIL_MIGRATIONS;
  const supported = schemaVersionOf(migrations);
  const database = openChecked(path, readonly, migrations, supported);
  if (!readonly) return database;
  let upgrade: string | undefined;
  try {
    upgrade = upgradeBeforeReading(path, database, migrations);
  } finally {
    if (upgrade !== undefined) database.$client.close();
  }
  if (upgrade === undefined) return database;
  try {
    openChecked(path, false, migrations, supported).$client.close();
  } catch (error) {
    if (!cannotWrite(error)) throw error;
    throw new Error(
      `${path} needs ${upgrade} before it can be read, and this process cannot write it: `
        + `${error instanceof Error ? error.message : String(error)}. Run a rerail command as the file's owner, `
        + "or make it writable for this user.",
      { cause: error },
    );
  }
  return openChecked(path, true, migrations, supported);
}

/**
 * What a writable open must do before this build reads the file a read-only `database` holds, or undefined when it
 * reads the file as it is. Throws, closing `database`, for a pending step that raises the floor.
 */
function upgradeBeforeReading(path: string, database: RerailDatabase, migrations: readonly Migration[]): string | undefined {
  try {
    const stamp = schemaStamp(database);
    if (stamp.version < RERAIL_BASELINE_SCHEMA_VERSION && stamp.version !== 1) return undefined;
    // A file stamped 1 passes the read-only open's checks only when adoptable; it has run no step.
    const pending = pendingMigrations(database, migrations);
    const ids = pending.map(migration => migration.id).join(", ");
    const raising = pending.filter(migration => migration.floor !== undefined).map(migration => migration.id);
    if (raising.length > 0) {
      throw new Error(
        `${path} needs this Rerail's schema steps (${ids}) before it can be read, and ${raising.join(", ")} raise its `
          + "schema floor, which a read-only open does not do. Start Rerail first (rerail daemon start), which runs them.",
      );
    }
    if (stamp.version === 1) return "adopting at the migration baseline";
    if (pending.length > 0) return `this Rerail's schema steps (${ids})`;
    // A table or index the DDL alone adds (no step needed) is also this build's shape.
    const missing = missingSchemaObjects(database);
    return missing.length === 0 ? undefined : `this Rerail's tables and indexes (${missing.join(", ")})`;
  } catch (error) {
    database.$client.close();
    throw error;
  }
}

/** The tables and indexes this build's DDL creates, by name. */
const SCHEMA_OBJECTS = RERAIL_SCHEMA_STATEMENTS
  .map(statement => /^CREATE (?:UNIQUE )?(?:TABLE|INDEX) IF NOT EXISTS `([^`]+)`/u.exec(statement)?.[1])
  .filter(name => name !== undefined);

/** The tables and indexes of this build's DDL that the file lacks. */
function missingSchemaObjects(database: RerailDatabase): string[] {
  const present = new Set(database.all<{ name: string }>(sql`SELECT name FROM sqlite_master`).map(row => row.name));
  return SCHEMA_OBJECTS.filter(name => !present.has(name));
}

function openChecked(path: string, readonly: boolean, migrations: readonly Migration[], supported: number): RerailDatabase {
  let database: RerailDatabase;
  try {
    database = connectSqlite(path, readonly);
  } catch (error) {
    throw notADatabase(error) ? new Error(unreadableDatabase(path, "not an SQLite database", supported)) : error;
  }
  try {
    if (!readonly) checkBeforeWriting(database, path, supported);
  } catch (error) {
    closeAndRethrow(database, path, error, supported);
  }
  try {
    applySchema(database, path, readonly ? undefined : migrations, supported);
  } catch (error) {
    if (!readonly) rollbackQuietly(database);
    closeAndRethrow(database, path, error, supported);
  }
  return database;
}

/** A writable open checks the stamp before it sets the file mode or switches the journal to WAL. */
function checkBeforeWriting(database: RerailDatabase, path: string, supported: number): void {
  // A plain read: the WAL switch cannot run inside a transaction, so the stamp is read again under the lock.
  const refusal = schemaRefusal(path, schemaStamp(database), supported);
  if (refusal !== undefined) throw new Error(refusal);
  prepareWriter(database, path);
}

/**
 * Migration steps, DDL and stamp run under one write lock, so concurrent openers agree on the file's shape and a
 * step that fails leaves the file as it was. `migrations` is undefined for a read-only open, which only checks.
 */
function applySchema(database: RerailDatabase, path: string, migrations: readonly Migration[] | undefined, supported: number): void {
  if (migrations !== undefined) database.run(sql.raw("BEGIN IMMEDIATE"));
  const stamp = schemaStamp(database);
  const refusal = schemaRefusal(path, stamp, supported);
  if (refusal !== undefined) throw new Error(refusal);
  if (migrations === undefined) return;
  const floor = migrate(database, { fresh: stamp.version === 0, floor: stamp.version }, RERAIL_SCHEMA_STATEMENTS, migrations);
  if (floor !== stamp.version) database.run(sql.raw(`PRAGMA user_version = ${floor}`));
  database.run(sql.raw("COMMIT"));
}

function rollbackQuietly(database: RerailDatabase): void {
  try {
    database.run(sql.raw("ROLLBACK"));
  } catch {
    // Already rolled back, or the connection failed before the transaction began.
  }
}

/** Closes the connection, then rethrows; a file that is not SQLite is reported as unreadable. */
function closeAndRethrow(database: RerailDatabase, path: string, error: unknown, supported: number): never {
  database.$client.close();
  throw notADatabase(error) ? new Error(unreadableDatabase(path, "not an SQLite database", supported)) : error;
}

/** Opens a read-only database lazily so viewers can start before the writer creates it. */
export class LazyReadonlyRerailDatabase {
  readonly path: string;
  #database: RerailDatabase | undefined;

  constructor(path: string) {
    this.path = path;
  }

  open(): RerailDatabase | undefined {
    if (this.#database !== undefined) return this.#database;
    if (!existsSync(this.path)) return undefined;
    this.#database = openRerailDatabase(this.path, { readonly: true });
    return this.#database;
  }

  close(): void {
    this.#database?.$client.close();
    this.#database = undefined;
  }
}

export interface HistoryResetReport {
  /** Ended sessions removed together with all of their history. */
  sessions: number;
  /** Sessions that were still open (`ended_at IS NULL`): their rows and runtime ownership stay, their history goes. */
  openSessions: number;
}

/**
 * `rerail reset`: delete one repository's execution history in ONE write transaction. Events,
 * work links, audits, audit requests, and repository audits of the repository's sessions go;
 * ended sessions go with them. Still-running sessions keep their row and runtime ownership
 * claim so live processes carry on recording. Rules, other repositories' history, audit
 * settings, and presentation preferences are untouched. The file is never unlinked or
 * rebuilt, so every connection holding it open keeps working.
 */
export function resetRepositoryHistory(path: string, repositoryId: string): HistoryResetReport {
  const database = openRerailDatabase(path);
  try {
    return database.transaction(() => {
      const counts = database.all<{ ended: number; open: number }>(sql`
        SELECT count(*) FILTER (WHERE ended_at IS NOT NULL) AS ended, count(*) FILTER (WHERE ended_at IS NULL) AS open
        FROM ${schema.sessions} WHERE ${schema.sessions.repositoryId} = ${repositoryId}
      `)[0]!;
      for (const table of schema.SESSION_HISTORY_TABLES) {
        database.run(sql`DELETE FROM ${table} WHERE ${table.sessionId} IN (${schema.sessionsOfRepository(repositoryId)})`);
      }
      const ended = schema.sessionsOfRepository(repositoryId, { ended: true });
      database.run(sql`DELETE FROM ${schema.policySessionOwners} WHERE ${schema.policySessionOwners.sessionId} IN (${ended})`);
      database.run(sql`DELETE FROM ${schema.sessions} WHERE ${schema.sessions.sessionId} IN (${ended})`);
      return { sessions: counts.ended, openSessions: counts.open };
    }, { behavior: "immediate" });
  } finally {
    database.$client.close();
  }
}
