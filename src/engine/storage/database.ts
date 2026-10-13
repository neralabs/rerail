import { chmodSync, existsSync, mkdirSync } from "node:fs";
import Database from "better-sqlite3";
import { dirname, join } from "node:path";
import { sql } from "drizzle-orm";
import { drizzle, type BetterSQLite3Database } from "drizzle-orm/better-sqlite3";
import * as schema from "./schema.js";
import { RERAIL_BASELINE_SCHEMA_VERSION, RERAIL_MIGRATIONS, migrate, schemaVersionOf, type Migration } from "./migrations.js";
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

/** A connection that has changed nothing on disk yet; a writable one creates the directory and an absent file. */
function connectSqlite(path: string, readonly: boolean): RerailDatabase {
  if (!readonly) mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  const client = new Database(path, { readonly });
  try {
    const database = drizzle(client, { schema });
    database.run(sql`PRAGMA busy_timeout = 5000`);
    return database;
  } catch (error) {
    client.close();
    throw error;
  }
}

/** What a writer owns: the file mode, WAL, and the connection's integrity settings. */
function prepareWriter(database: RerailDatabase, path: string): void {
  chmodSync(path, 0o600);
  enableWal(() => database.run(sql`PRAGMA journal_mode = WAL`));
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
}

function schemaStamp(database: RerailDatabase): SchemaStamp {
  const tables = database.all<{ name: string }>(sql`SELECT name FROM sqlite_master WHERE type = 'table'`);
  const version = database.all<{ user_version: number }>(sql`PRAGMA user_version`)[0]?.user_version ?? 0;
  return { empty: tables.length === 0, version };
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

/**
 * Why a build reading schemas up to `supported` cannot open a file stamped `stamp`, or undefined when it can: a
 * file stamped by a newer Rerail, a file that is not new and carries no Rerail stamp (a `user_version` below 1),
 * or a file from before the migration baseline. Only a file with neither a table nor a stamp is new; a stamped
 * file is judged by its stamp even when it holds no table.
 */
function schemaRefusal(path: string, stamp: SchemaStamp, supported: number): string | undefined {
  if (stamp.version > supported) {
    return `${path} was written by a newer Rerail (schema v${stamp.version}); this Rerail supports schema v${supported}. `
      + "Update Rerail to open it, or move the file aside to start fresh.";
  }
  if (stamp.version === 0 && stamp.empty) return undefined;
  if (stamp.version < 1) return unreadableDatabase(path, "no Rerail schema version", supported);
  if (stamp.version < RERAIL_BASELINE_SCHEMA_VERSION) {
    return unreadableDatabase(path, `schema v${stamp.version}, from a development build before the migration baseline`, supported);
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
 * not run and the current DDL, which is idempotent, and stamps the floor they reach (`migrations.ts`). A
 * read-only open changes nothing, so it reads a file a writer has not migrated yet in its old shape; a read-only
 * open of a file whose creator has not committed the schema yet sees no table at all.
 */
export function openRerailDatabase(path: string, options: OpenRerailDatabaseOptions = {}): RerailDatabase {
  const readonly = options.readonly === true;
  const migrations = options.migrations ?? RERAIL_MIGRATIONS;
  const supported = schemaVersionOf(migrations);
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
