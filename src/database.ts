import { randomUUID } from "node:crypto";
import { chmodSync, existsSync, linkSync, mkdirSync, renameSync, rmSync } from "node:fs";
import Database from "better-sqlite3";
import { basename, dirname, join, resolve } from "node:path";
import { sql } from "drizzle-orm";
import { drizzle, type BetterSQLite3Database } from "drizzle-orm/better-sqlite3";
import * as schema from "./schema.js";
import { RERAIL_SCHEMA_STATEMENTS } from "./schema-sql.js";
import { enableWal } from "./sqlite-wal.js";
import { defaultRerailHome } from "./rerail-home.js";

export type RerailDatabase = BetterSQLite3Database<typeof schema> & { readonly $client: Database.Database };

export interface OpenRerailDatabaseOptions {
  /** Open an existing database without write access; the schema is not applied. */
  readonly?: boolean;
}

const LEGACY_TABLES = ["transcript_sessions", "transcript_events", "policy_replay_events", "policy_program_artifacts"];

/** The one Rerail database: rules and every repository's history, in the Rerail home. */
export const RERAIL_DATABASE_FILE = "rerail.sqlite";
/** The machine rule catalog before rules and history shared a file; adopted once, then left in place. */
export const LEGACY_RULE_CATALOG_FILE = "rules.sqlite";
/** A checkout's history before it moved into the Rerail home; its legacy `rules` table is still imported. */
export const LEGACY_CHECKOUT_DATABASE = join(".rerail", "transcripts.sqlite");

/** `$RERAIL_HOME/rerail.sqlite`, with the Rerail home defaulting to `~/.rerail`. */
export function defaultRerailDatabasePath(environment: NodeJS.ProcessEnv = process.env): string {
  return join(defaultRerailHome(environment), RERAIL_DATABASE_FILE);
}

/** Where a checkout kept its own database before the Rerail home held all history. */
export function legacyCheckoutDatabasePath(workspaceRoot: string): string {
  return join(resolve(workspaceRoot), LEGACY_CHECKOUT_DATABASE);
}

const SCHEMA_REVISION_TABLE = "rerail_schema_revision";
const SCHEMA_REVISION_TABLE_DDL =
  "CREATE TABLE IF NOT EXISTS `rerail_schema_revision` (singleton INTEGER PRIMARY KEY CHECK(singleton = 1), revision INTEGER NOT NULL)";

function recordSchemaRevision(database: RerailDatabase, revision: number): void {
  database.run(sql.raw(SCHEMA_REVISION_TABLE_DDL));
  database.run(sql.raw(
    `INSERT INTO \`${SCHEMA_REVISION_TABLE}\` (singleton, revision) VALUES (1, ${revision}) `
    + "ON CONFLICT(singleton) DO UPDATE SET revision = MAX(revision, excluded.revision)",
  ));
}

/**
 * Additive steps from an older stamped version to the next one. Each entry
 * runs before the current DDL is (re)applied. Version 0 is a v1 database
 * written before the stamp existed.
 */
type SchemaUpgrade = (database: RerailDatabase) => void;

/** Upgrade either historical kind-based shape without disturbing an already-current table. */
const migratePolicyExecutionAxes: SchemaUpgrade = database => {
  const columns = database.all<{ name: string }>(sql`PRAGMA table_info(policy_runs)`).map(row => row.name);
  if (!columns.includes("kind")) return;
  database.run(sql.raw("ALTER TABLE `policy_runs` RENAME TO `policy_runs_legacy`"));
  const statement = RERAIL_SCHEMA_STATEMENTS.find(candidate => candidate.startsWith("CREATE TABLE IF NOT EXISTS `policy_runs`"));
  if (statement === undefined) throw new Error("Rerail schema is missing policy_runs DDL");
  database.run(sql.raw(statement));
  database.run(sql.raw(
    "INSERT INTO `policy_runs` (" +
      "`run_id`, `session_id`, `workspace_root`, `scope`, `mode`, `revision`, `rule_provenance_json`, `paths_json`, " +
      "`started_at`, `finished_at`, `status`, `findings_json`, `failures_json`" +
    ") SELECT " +
      "`run_id`, `session_id`, `workspace_root`, " +
      "CASE `kind` " +
        "WHEN 'full-scan' THEN 'repository' WHEN 'scoped-scan' THEN 'touched-files' " +
        "WHEN 'ratchet' THEN 'ai-output' WHEN 'shadow' THEN 'repository' ELSE 'touched-files' END, " +
      "CASE `kind` WHEN 'generation' THEN 'resteer' ELSE 'log' END, " +
      "`revision`, '[]', `scope_json`, `started_at`, `finished_at`, `status`, `findings_json`, `failures_json` " +
    "FROM `policy_runs_legacy` WHERE `kind` <> 'delivery'",
  ));
  database.run(sql.raw("DROP TABLE `policy_runs_legacy`"));
};

/**
 * Both schema v7 lines existed independently before the branches merged: main
 * added scoped-rule provenance while the scan-control branch added immutable
 * catalog revisions and repository diagnostics. Every entry is idempotent so
 * databases stamped by either line converge on v9.
 */
const migrateMergedPolicyMetadata: SchemaUpgrade = database => {
  const columns = new Set(database.all<{ name: string }>(sql`PRAGMA table_info(policy_runs)`).map(row => row.name));
  if (!columns.has("catalog_revision")) database.run(sql.raw("ALTER TABLE `policy_runs` ADD COLUMN `catalog_revision` text"));
  if (!columns.has("rule_revisions_json"))
    database.run(sql.raw("ALTER TABLE `policy_runs` ADD COLUMN `rule_revisions_json` text NOT NULL DEFAULT '[]'"));
  if (!columns.has("rule_provenance_json"))
    database.run(sql.raw("ALTER TABLE `policy_runs` ADD COLUMN `rule_provenance_json` text NOT NULL DEFAULT '[]'"));

  const tables = new Set(database.all<{ name: string }>(sql`SELECT name FROM sqlite_master WHERE type = 'table'`).map(row => row.name));
  if (tables.has("policy_settings"))
    database.run(sql.raw("UPDATE `policy_settings` SET `settings_json` = json_remove(`settings_json`, '$.initialMode')"));
  const statement = RERAIL_SCHEMA_STATEMENTS.find(candidate => candidate.startsWith("CREATE TABLE IF NOT EXISTS `policy_repository_diagnostics`"));
  if (statement === undefined) throw new Error("Rerail schema is missing policy_repository_diagnostics DDL");
  database.run(sql.raw(statement));
  database.run(sql.raw(
    "INSERT OR IGNORE INTO `policy_repository_diagnostics` " +
      "(`workspace_root`, `session_id`, `run_id`, `status`, `started_at`, `finished_at`, `error`) " +
    "SELECT p.`workspace_root`, p.`session_id`, p.`run_id`, " +
      "CASE p.`status` WHEN 'running' THEN 'interrupted' WHEN 'superseded' THEN 'interrupted' ELSE p.`status` END, " +
      "p.`started_at`, COALESCE(p.`finished_at`, " + Date.now() + "), " +
      "CASE WHEN p.`status` IN ('running', 'superseded') THEN 'Historic repository diagnostic did not complete' ELSE NULL END " +
    "FROM `policy_runs` p WHERE p.`scope` = 'repository' AND p.`mode` = 'log' AND NOT EXISTS (" +
      "SELECT 1 FROM `policy_runs` earlier WHERE earlier.`workspace_root` = p.`workspace_root` " +
        "AND earlier.`scope` = 'repository' AND earlier.`mode` = 'log' " +
        "AND (earlier.`started_at` < p.`started_at` OR (earlier.`started_at` = p.`started_at` AND earlier.`run_id` < p.`run_id`))" +
    ")",
  ));
};

const SCHEMA_UPGRADES: Readonly<Record<number, SchemaUpgrade>> = {
  1: database => {
    // A v1 file another process already widened must not fail here; SQLite has no ADD COLUMN IF NOT EXISTS.
    const columns = new Set(database.all<{ name: string }>(sql`PRAGMA table_info(rules)`).map(row => row.name));
    if (!columns.has("source_digest")) database.run(sql.raw("ALTER TABLE `rules` ADD COLUMN `source_digest` text"));
  },
  2: database => {
    // v3 compiles the enabled policy rows in memory; the persisted catalog cache is gone.
    database.run(sql.raw("DROP TABLE IF EXISTS `policy_dsl_rules`"));
    database.run(sql.raw("DROP TABLE IF EXISTS `policy_dsl_catalog`"));
  },
  3: database => {
    // v4 adds independent execution history. It intentionally has no session
    // foreign key, so a policy run can outlive/reset independently of sessions.
    const statement = RERAIL_SCHEMA_STATEMENTS.find(candidate => candidate.startsWith("CREATE TABLE IF NOT EXISTS `policy_runs`"));
    if (statement === undefined) throw new Error("Rerail schema is missing policy_runs DDL");
    database.run(sql.raw(statement));
  },
  4: migratePolicyExecutionAxes,
  5: migratePolicyExecutionAxes,
  6: migrateMergedPolicyMetadata,
  7: migrateMergedPolicyMetadata,
  8: migrateMergedPolicyMetadata,
  9: database => {
    const statement = RERAIL_SCHEMA_STATEMENTS.find(candidate => candidate.startsWith("CREATE TABLE IF NOT EXISTS `presentation_preferences`"));
    if (statement === undefined) throw new Error("Rerail schema is missing presentation_preferences DDL");
    database.run(sql.raw(statement));
  },
  10: database => {
    const statement = RERAIL_SCHEMA_STATEMENTS.find(candidate =>
      candidate.startsWith("CREATE TABLE IF NOT EXISTS `policy_run_result_pages`")
    );
    if (statement === undefined) throw new Error("Rerail schema is missing policy result page DDL");
    database.run(sql.raw(statement));
  },
  11: database => {
    // v12 lets a structural cosine row omit its reference; SQLite can change a CHECK only by rebuilding the table.
    // It runs after result pages so a floor-stamped revision-11 file, which already has them, still gets the rebuild.
    const statement = RERAIL_SCHEMA_STATEMENTS.find(candidate => candidate.startsWith("CREATE TABLE IF NOT EXISTS `rules`"));
    if (statement === undefined) throw new Error("Rerail schema is missing rules DDL");
    database.run(sql.raw("ALTER TABLE `rules` RENAME TO `rules_legacy_v11`"));
    database.run(sql.raw(statement));
    const legacy = new Set(database.all<{ name: string }>(sql`PRAGMA table_info(rules_legacy_v11)`).map(row => row.name));
    const columns = database.all<{ name: string }>(sql`PRAGMA table_info(rules)`)
      .filter(row => legacy.has(row.name))
      .map(row => `\`${row.name}\``)
      .join(", ");
    database.run(sql.raw(`INSERT INTO \`rules\` (${columns}) SELECT ${columns} FROM \`rules_legacy_v11\``));
    // Dropping the old table drops its index too; the current DDL recreates it after the upgrades.
    database.run(sql.raw("DROP TABLE `rules_legacy_v11`"));
  },
  12: database => {
    // v13 attributes sessions to a repository now that every repository shares one database. A file
    // without a sessions table gets the column from the current DDL instead.
    const columns = new Set(database.all<{ name: string }>(sql`PRAGMA table_info(sessions)`).map(row => row.name));
    if (columns.size > 0 && !columns.has("repository_id")) database.run(sql.raw("ALTER TABLE `sessions` ADD COLUMN `repository_id` text"));
  },
};

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

/** Move a pre-schema database (and its WAL/shm sidecars) aside so the caller can start fresh. */
function archiveLegacyDatabase(path: string): string {
  const archived = `${path}.pre-schema-${new Date().toISOString().replace(/[-:]/gu, "").replace(/\.\d+Z$/u, "Z")}`;
  renameSync(path, archived);
  for (const suffix of ["-wal", "-shm"]) {
    if (existsSync(`${path}${suffix}`)) renameSync(`${path}${suffix}`, `${archived}${suffix}`);
  }
  return archived;
}

function schemaRevisionOnDisk(
  database: RerailDatabase,
  userVersion: number,
  hasRevisionTable: boolean,
  unstampedPreFloor: boolean,
): number {
  if (!hasRevisionTable || userVersion < schema.RERAIL_SCHEMA_COMPATIBILITY_FLOOR) {
    return unstampedPreFloor ? 10 : userVersion;
  }
  return database.all<{ revision: number }>(sql.raw(`SELECT revision FROM \`${SCHEMA_REVISION_TABLE}\` WHERE singleton = 1`))[0]?.revision ?? userVersion;
}

/**
 * A `rerail.sqlite` that does not exist yet beside a legacy `rules.sqlite` starts as a
 * transactionally consistent copy of it (`VACUUM INTO`), published with a no-clobber link so
 * concurrent openers agree on one copy. The legacy file is only read and is left in place;
 * once `rerail.sqlite` exists it is never consulted again.
 */
function adoptLegacyRuleCatalog(path: string): void {
  if (basename(path) !== RERAIL_DATABASE_FILE || existsSync(path)) return;
  const legacy = join(dirname(path), LEGACY_RULE_CATALOG_FILE);
  if (!existsSync(legacy)) return;
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  const temporary = `${path}.${process.pid}.${randomUUID()}.adopt`;
  try {
    const source = new Database(legacy, { readonly: true, fileMustExist: true });
    try {
      source.pragma("busy_timeout = 5000");
      source.prepare("VACUUM INTO ?").run(temporary);
    } finally {
      source.close();
    }
    try {
      chmodSync(temporary, 0o600);
      linkSync(temporary, path);
      process.stderr.write(`rerail: copied the rule catalog from ${legacy} into ${path}; ${legacy} is no longer read\n`);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
    }
  } finally {
    // A failed VACUUM INTO can leave a partial copy, or none at all.
    rmSync(temporary, { force: true });
  }
}

/**
 * Open the project database, creating the directory, file, and schema as needed.
 * Writers own the file mode (0600) and directory mode (0700) they create.
 *
 * A database written before schema v1 is renamed to `<path>.pre-schema-<timestamp>`
 * and a fresh one takes its place: a session must not fail to start because of an
 * old file, and the archive keeps the old transcripts readable. `PRAGMA user_version`
 * records the oldest compatible reader; additive revisions are recorded separately
 * and reached through `SCHEMA_UPGRADES`.
 */
export function openRerailDatabase(path: string, options: OpenRerailDatabaseOptions = {}): RerailDatabase {
  if (options.readonly !== true) adoptLegacyRuleCatalog(path);
  const database = openSqliteDatabase(path, options);
  if (options.readonly === true) return database;
  // Inspection, upgrade, DDL, and stamp run under one write lock: two processes opening a
  // v1 file at once must not both see the column missing and both try to add it.
  database.run(sql.raw("BEGIN IMMEDIATE"));
  try {
    const tables = database.all<{ name: string }>(sql`SELECT name FROM sqlite_master WHERE type = 'table'`).map(row => row.name);
    const legacy = LEGACY_TABLES.filter(name => tables.includes(name));
    if (legacy.length > 0) {
      database.run(sql.raw("ROLLBACK"));
      database.$client.close();
      const archived = archiveLegacyDatabase(path);
      process.stderr.write(
        `rerail: ${path} predates schema v${schema.RERAIL_SCHEMA_VERSION} (found ${legacy.join(", ")}); moved it to ${archived} and started a fresh database\n`,
      );
      return openRerailDatabase(path, options);
    }
    const userVersion = database.all<{ user_version: number }>(sql`PRAGMA user_version`)[0]?.user_version ?? 0;
    const hasRevisionTable = tables.includes(SCHEMA_REVISION_TABLE);
    // Before the compatibility-floor stamp, parallel lines reused version numbers: v11 added either
    // structural cosine rows or result pages, and an unreleased single-database build stamped v12 for
    // repository attribution on top of the cosine line. Replay the idempotent upgrades from 10 so any
    // of those histories converges on the current revision under the shared floor.
    const unstampedPreFloor = (userVersion === 11 || userVersion === 12) && !hasRevisionTable;
    const floorOnDisk = unstampedPreFloor ? 10 : userVersion;
    let revisionOnDisk = schemaRevisionOnDisk(database, userVersion, hasRevisionTable, unstampedPreFloor);
    // An empty file needs only the DDL; an unstamped file with tables is v1, written before the stamp existed.
    if (tables.length === 0) revisionOnDisk = schema.RERAIL_SCHEMA_VERSION;
    else if (revisionOnDisk === 0) revisionOnDisk = 1;
    if (floorOnDisk > schema.RERAIL_SCHEMA_VERSION) {
      throw new Error(`${path} requires a newer Rerail: its compatibility floor is v${floorOnDisk}, this Rerail is v${schema.RERAIL_SCHEMA_VERSION}`);
    }
    for (let step = revisionOnDisk; step < schema.RERAIL_SCHEMA_VERSION; step += 1) {
      SCHEMA_UPGRADES[step]?.(database);
    }
    for (const statement of RERAIL_SCHEMA_STATEMENTS) database.run(sql.raw(statement));
    recordSchemaRevision(database, Math.max(revisionOnDisk, schema.RERAIL_SCHEMA_VERSION));
    database.run(sql.raw(`PRAGMA user_version = ${Math.max(floorOnDisk, schema.RERAIL_SCHEMA_COMPATIBILITY_FLOOR)}`));
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
 * work links, policy runs, scan requests, and repository diagnostics of the repository's
 * sessions go; ended sessions go with them. Still-running sessions keep their row and runtime
 * ownership claim so live processes carry on recording. Rules, other repositories' history,
 * policy settings, and presentation preferences are untouched. The file is never unlinked or
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
