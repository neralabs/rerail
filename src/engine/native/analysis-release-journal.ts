import { existsSync } from "node:fs";
import { sql } from "drizzle-orm";
import { openSqliteDatabase, type RerailDatabase } from "../storage/database.js";

type Connection = Pick<RerailDatabase, "all" | "get" | "run">;

export function releaseCutoff(id: string): number | undefined {
  const colon = id.indexOf(":");
  if (colon < 0) return undefined;
  const prefix = id.slice(0, colon);
  const cutoff = Number(prefix);
  if (!/^\d+$/u.test(prefix) || !Number.isSafeInteger(cutoff)) throw new Error("Invalid analysis scope release cutoff");
  return cutoff;
}

/** The attached decision log uses the rollback journal so its SQLite connection uses FULL synchronization. */
export function attachReleaseJournal(db: Connection, path: string): void {
  if (db.get(sql`SELECT 1 FROM temp.sqlite_schema WHERE name='analysis_journal_release_insert'`)) return;
  if (!db.all<{ name: string }>(sql`PRAGMA database_list`).some(row => row.name === "release_journal")) {
    openSqliteDatabase(path, { journalMode: "delete" }).$client.close();
    db.run(sql`ATTACH DATABASE ${path} AS release_journal`);
  }
  db.run(sql`CREATE TABLE IF NOT EXISTS release_journal.analysis_journal_generation (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1), generation INTEGER NOT NULL) STRICT`);
  db.run(sql`CREATE TABLE IF NOT EXISTS release_journal.analysis_journal_release (
    id TEXT PRIMARY KEY, directory TEXT NOT NULL, next_attempt_at INTEGER NOT NULL) STRICT`);
  db.run(sql`CREATE TABLE IF NOT EXISTS release_journal.analysis_journal_probe (
    release_id TEXT NOT NULL, directory TEXT NOT NULL, store TEXT NOT NULL, next_attempt_at INTEGER NOT NULL,
    attempts INTEGER NOT NULL, last_error TEXT, PRIMARY KEY(release_id,store)) STRICT`);
  db.run(sql`CREATE TABLE IF NOT EXISTS release_journal.analysis_journal_target (
    release_id TEXT NOT NULL REFERENCES analysis_journal_release(id) ON DELETE CASCADE,
    directory TEXT NOT NULL, path TEXT NOT NULL, identity TEXT NOT NULL, probe INTEGER NOT NULL,
    PRIMARY KEY(release_id,directory,path,identity,probe)) STRICT`);
  db.run(sql`CREATE TABLE IF NOT EXISTS release_journal.analysis_retired_release (
    identity TEXT NOT NULL, directory TEXT NOT NULL, release_id TEXT NOT NULL,
    PRIMARY KEY(identity,directory,release_id)) STRICT`);
  // A later stop of the same directory and home covers the earlier cutoff. Older receipt formats stay independent.
  db.run(sql`CREATE TRIGGER IF NOT EXISTS release_journal.analysis_retired_release_compact
    AFTER INSERT ON analysis_retired_release WHEN NEW.release_id GLOB '[0-9]*:j-*:*' BEGIN
    DELETE FROM analysis_retired_release WHERE identity=NEW.identity AND directory=NEW.directory
      AND release_id GLOB '[0-9]*:j-*:*' AND substr(release_id,-64)=substr(NEW.release_id,-64)
      AND release_id!=(SELECT release_id FROM analysis_retired_release WHERE identity=NEW.identity AND directory=NEW.directory
        AND release_id GLOB '[0-9]*:j-*:*' AND substr(release_id,-64)=substr(NEW.release_id,-64)
        ORDER BY CAST(release_id AS INTEGER) DESC,release_id DESC LIMIT 1);
    END`);
  for (const queue of queues) {
    const { table, journal, columns, keys } = queue;
    const write = `INSERT INTO ${journal} (${columns.join(",")}) VALUES (${columns.map(c => "NEW." + c).join(",")})
      ON CONFLICT (${keys.join(",")}) DO UPDATE SET ${columns.filter(c => !keys.includes(c)).map(c => c + "=excluded." + c).join(",")}`;
    for (const event of ["insert", "update"]) {
      db.run(sql.raw(`CREATE TEMP TRIGGER ${journal}_${event} AFTER ${event} ON main.${table}
        BEGIN ${write}; END`));
    }
    db.run(sql.raw(`CREATE TEMP TRIGGER ${journal}_delete AFTER DELETE ON main.${table}
      BEGIN DELETE FROM ${journal} WHERE ${keys.map(c => c + "=OLD." + c).join(" AND ")}; END`));
  }
}

// These lists also describe how to restore an older control database. Legacy receipts still use its own queue.
const queues = [
  { table: "analysis_scope_releases", journal: "analysis_journal_release", keys: ["id"], columns: ["id", "directory", "next_attempt_at"] },
  { table: "analysis_scope_release_probes", journal: "analysis_journal_probe", keys: ["release_id", "store"],
    columns: ["release_id", "directory", "store", "next_attempt_at", "attempts", "last_error"] },
];

export function journalHasWork(path: string): boolean {
  if (!existsSync(path)) return false;
  const db = openSqliteDatabase(path, { readonly: true });
  try {
    return Boolean(db.get(sql`SELECT 1 FROM sqlite_schema WHERE name='analysis_journal_release'`) &&
      db.get(sql`SELECT 1 WHERE EXISTS(SELECT 1 FROM analysis_journal_release) OR EXISTS(SELECT 1 FROM analysis_journal_probe)`));
  } finally { db.$client.close(); }
}

/** The journal is authoritative for new receipts, including a crash between the control and journal commits. */
export function recoverReleaseJournal(db: Connection): void {
  for (const { table, journal, columns, keys } of queues) {
    const id = keys[0];
    const matches = keys.map(c => `j.${c}=${table}.${c}`).join(" AND ");
    // A pending receipt from an older version first enters the journal when the worker opens the old queue.
    db.run(sql.raw(`INSERT OR IGNORE INTO release_journal.${journal} (${columns.join(",")})
      SELECT ${columns.join(",")} FROM main.${table} WHERE ${id} NOT GLOB '[0-9]*:j-*:*'`));
    db.run(sql.raw(`DELETE FROM main.${table} WHERE ${id} GLOB '[0-9]*:j-*:*' AND
      NOT EXISTS (SELECT 1 FROM release_journal.${journal} j WHERE ${matches})`));
    db.run(sql.raw(`INSERT OR IGNORE INTO main.${table} (${columns.join(",")})
      SELECT ${columns.join(",")} FROM release_journal.${journal}`));
  }
  db.run(sql`INSERT INTO main.analysis_scope_generation(singleton,generation)
    SELECT singleton,generation FROM release_journal.analysis_journal_generation WHERE singleton=1
    ON CONFLICT(singleton) DO UPDATE SET generation=max(generation,excluded.generation)`);
}
