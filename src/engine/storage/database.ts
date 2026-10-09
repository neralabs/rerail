import { chmodSync, existsSync, mkdirSync } from "node:fs";
import Database from "better-sqlite3";
import { dirname, join } from "node:path";
import { sql } from "drizzle-orm";
import { drizzle, type BetterSQLite3Database } from "drizzle-orm/better-sqlite3";
import * as schema from "./schema.js";
import { RERAIL_SCHEMA_STATEMENTS } from "./schema-sql.generated.js";
import { enableWal } from "./sqlite-wal.js";
import { defaultRerailHome } from "./rerail-home.js";

export type RerailDatabase = BetterSQLite3Database<typeof schema> & { readonly $client: Database.Database };

export interface OpenRerailDatabaseOptions {
  /** Open an existing database without write access; the schema is not applied. */
  readonly?: boolean;
}

/** The one Rerail database: rules and every repository's history, in the Rerail home. */
export const RERAIL_DATABASE_FILE = "rerail.sqlite";

/** `$RERAIL_HOME/rerail.sqlite`, with the Rerail home defaulting to `~/.rerail`. */
export function defaultRerailDatabasePath(environment: NodeJS.ProcessEnv = process.env): string {
  return join(defaultRerailHome(environment), RERAIL_DATABASE_FILE);
}

/** Open the Node-owned SQLite database without applying Rerail's project schema. */
export function openSqliteDatabase(path: string, options: OpenRerailDatabaseOptions = {}): RerailDatabase {
  const readonly = options.readonly === true;
  if (!readonly) mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  const client = new Database(path, { readonly });
  const database = drizzle(client, { schema });
  database.run(sql`PRAGMA busy_timeout = 5000`);
  if (!readonly) {
    chmodSync(path, 0o600);
    enableWal(() => database.run(sql`PRAGMA journal_mode = WAL`));
    database.run(sql`PRAGMA foreign_keys = ON`);
    database.run(sql`PRAGMA synchronous = FULL`);
  }
  return database;
}

/**
 * Open the Rerail database, creating the directory, file, and schema as needed.
 * Writers own the file mode (0600) and directory mode (0700) they create.
 *
 * `PRAGMA user_version` holds the schema version the file was written for. A file stamped by a
 * newer Rerail is refused; otherwise the current DDL, which is idempotent, is applied and the
 * file is stamped with this build's version.
 */
export function openRerailDatabase(path: string, options: OpenRerailDatabaseOptions = {}): RerailDatabase {
  const database = openSqliteDatabase(path, options);
  if (options.readonly === true) return database;
  // DDL and stamp run under one write lock, so concurrent openers agree on the file's shape.
  database.run(sql.raw("BEGIN IMMEDIATE"));
  try {
    const version = database.all<{ user_version: number }>(sql`PRAGMA user_version`)[0]?.user_version ?? 0;
    if (version > schema.RERAIL_SCHEMA_VERSION) {
      throw new Error(
        `${path} was written by a newer Rerail (schema v${version}); this Rerail supports schema v${schema.RERAIL_SCHEMA_VERSION}. Update Rerail to open it.`,
      );
    }
    for (const statement of RERAIL_SCHEMA_STATEMENTS) database.run(sql.raw(statement));
    if (version < schema.RERAIL_SCHEMA_VERSION) database.run(sql.raw(`PRAGMA user_version = ${schema.RERAIL_SCHEMA_VERSION}`));
    database.run(sql.raw("COMMIT"));
  } catch (error) {
    try {
      database.run(sql.raw("ROLLBACK"));
    } catch {
      // Already rolled back, or the connection failed before the transaction began.
    }
    database.$client.close();
    throw error;
  }
  return database;
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
