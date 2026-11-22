import { existsSync } from "node:fs";
import { sep, toNamespacedPath } from "node:path";
import { storedPath } from "../catalog/checkout-control.js";
import { openRerailDatabase, openSqliteDatabase, type RerailDatabase } from "../storage/database.js";
import { attachReleaseJournal, recoverReleaseJournal, releaseCutoff } from "./analysis-release-journal.js";
import { assertContentStoreOwner } from "./main-instance.js";
import { loadNativeBinding } from "./native.js";

export function storeRegistryPath(database: string): string { return `${storedPath(database)}.analysis-stores.sqlite`; }
export function releaseJournalPath(database: string): string { return `${storedPath(database)}.analysis-releases.sqlite`; }

type Client = RerailDatabase["$client"];
interface StorePath { identity: string; path: string }
interface Owner { directory: string; first_generation: number; last_generation: number }
function tables(db: Client, schema: "main" | "routing"): Set<string> {
  return new Set(db.prepare(`SELECT name FROM ${schema}.sqlite_schema WHERE type='table'`).pluck().all() as string[]);
}

export function listAnalysisStores(database: string): StorePath[] {
  const path = storeRegistryPath(database);
  if (!existsSync(path)) return [];
  const index = openSqliteDatabase(path, { readonly: true }).$client;
  try { return activePaths(index, "main"); } finally { index.close(); }
}

function activePaths(db: Client, schema: "main" | "routing", identity?: string): StorePath[] {
  const present = tables(db, schema);
  const sources = ["analysis_maintenance_identity", "analysis_directory_store_identity", "analysis_named_store", "analysis_named_acquisition"].filter(t => present.has(t));
  if (!sources.length) return [];
  const select = sources.map(t => `SELECT identity,path FROM ${schema}.${t}`).join(" UNION ");
  const retired = present.has("analysis_retired_route") ? ` AND NOT EXISTS(
    SELECT 1 FROM ${schema}.analysis_retired_route retired WHERE retired.identity=r.identity AND retired.path=r.path)` : "";
  return db.prepare(`SELECT identity,path FROM (${select}) r WHERE identity IS NOT NULL${identity === undefined ? "" : " AND identity=?"}${retired}
    ORDER BY identity,path`).all(...(identity === undefined ? [] : [identity])) as StorePath[];
}

/** Serialize retirement and reactivation with a committed directory stop through the control writer. */
function changeRoutes<T>(database: string, operation: (db: Client) => T): T {
  const control = openRerailDatabase(database);
  const db = control.$client;
  try {
    attachReleaseJournal(control, releaseJournalPath(database));
    db.prepare("ATTACH DATABASE ? AS routing").run(storeRegistryPath(database));
    return db.transaction(() => {
      recoverReleaseJournal(control);
      db.exec(`CREATE TABLE IF NOT EXISTS routing.analysis_retired_route (
        identity TEXT NOT NULL, path TEXT NOT NULL, PRIMARY KEY(identity,path)) STRICT;
        CREATE INDEX IF NOT EXISTS routing.analysis_retired_route_path ON analysis_retired_route(path);
        CREATE TABLE IF NOT EXISTS routing.analysis_retired_directory (
        identity TEXT NOT NULL, directory TEXT NOT NULL, first_generation INTEGER NOT NULL, last_generation INTEGER NOT NULL,
        PRIMARY KEY(identity,directory)) STRICT;
        CREATE INDEX IF NOT EXISTS routing.analysis_retired_directory_root ON analysis_retired_directory(directory)`);
      return operation(db);
    }).immediate();
  } finally { db.close(); }
}

function under(directory: string, root: string): boolean {
  return directory === root || directory.startsWith(root.endsWith(sep) ? root : root + sep);
}

export function retireAnalysisStore(database: string, identity: string): number {
  if (!existsSync(storeRegistryPath(database))) return 0;
  return changeRoutes(database, db => {
    const paths = activePaths(db, "routing", identity);
    if (!paths.length) return 0;
    const present = tables(db, "routing");
    const knownDirectories = present.has("analysis_directory_store_identity");
    if (knownDirectories) {
      db.prepare(`INSERT INTO routing.analysis_retired_directory SELECT identity,directory,min(first_generation),max(last_generation)
        FROM routing.analysis_directory_store_identity WHERE identity=? GROUP BY identity,directory
        ON CONFLICT(identity,directory) DO UPDATE SET first_generation=min(first_generation,excluded.first_generation),
        last_generation=max(last_generation,excluded.last_generation)`).run(identity);
      db.prepare(`DELETE FROM routing.analysis_directory_store AS d WHERE EXISTS (
        SELECT 1 FROM routing.analysis_directory_store_identity i WHERE i.path=d.path AND i.directory=d.directory AND i.identity=?)
        AND NOT EXISTS (SELECT 1 FROM routing.analysis_directory_store_identity i WHERE i.path=d.path AND i.directory=d.directory AND i.identity!=?)`)
        .run(identity, identity);
      db.prepare("DELETE FROM routing.analysis_directory_store_identity WHERE identity=?").run(identity);
    }
    if (present.has("analysis_maintenance_identity")) db.prepare("DELETE FROM routing.analysis_maintenance_identity WHERE identity=?").run(identity);
    const remember = db.prepare("INSERT OR IGNORE INTO routing.analysis_retired_route VALUES (?,?)");
    for (const { path } of paths) {
      remember.run(identity, path);
      if (present.has("analysis_maintenance_identity") && present.has("analysis_maintenance_store")) {
        db.prepare(`DELETE FROM routing.analysis_maintenance_store WHERE path=? AND NOT EXISTS (
          SELECT 1 FROM routing.analysis_maintenance_identity WHERE path=?)`).run(path, path);
      }
    }
    // A retired receipt lives independently of the active delivery queue and its cascading deletes.
    db.prepare(`INSERT OR IGNORE INTO release_journal.analysis_retired_release
      SELECT DISTINCT t.identity,r.directory,r.id FROM release_journal.analysis_journal_target t
      JOIN release_journal.analysis_journal_release r ON r.id=t.release_id WHERE t.identity=?`).run(identity);
    db.prepare("DELETE FROM release_journal.analysis_journal_target WHERE identity=?").run(identity);
    // Older stops selected their targets from the live registry each time.
    const owners = db.prepare("SELECT directory,first_generation FROM routing.analysis_retired_directory WHERE identity=?").all(identity) as Owner[];
    const legacy = db.prepare("SELECT id,directory FROM release_journal.analysis_journal_release WHERE id NOT GLOB '[0-9]*:j-*:*'").all() as { id: string; directory: string }[];
    for (const { id, directory } of legacy) {
      const cutoff = releaseCutoff(id);
      if (owners.some(owner => under(owner.directory, directory) && (cutoff === undefined || owner.first_generation <= cutoff))) {
        db.prepare("INSERT OR IGNORE INTO release_journal.analysis_retired_release VALUES (?,?,?)").run(identity, directory, id);
      }
    }
    // A fallback receipt predates identification of its route. Transfer it only when this
    // store owned the stopped directory and no other active identity uses this path.
    const active = new Set(activePaths(db, "routing").map(route => route.path));
    for (const { path } of paths) {
      if (active.has(path)) continue;
      // Old registries tracked a path without an identity. Keep it if it still serves an unidentified directory.
      if (present.has("analysis_store") && !(present.has("analysis_directory_store") &&
          db.prepare("SELECT 1 FROM routing.analysis_directory_store WHERE path=? LIMIT 1").get(path))) {
        db.prepare("DELETE FROM routing.analysis_store WHERE path=?").run(path);
      }
      const probes = db.prepare(`SELECT release_id,directory FROM analysis_scope_release_probes WHERE store=?
        UNION SELECT t.release_id,r.directory FROM release_journal.analysis_journal_target t
          JOIN release_journal.analysis_journal_release r ON r.id=t.release_id WHERE t.path=? AND t.probe=1`).all(path, path) as { release_id: string; directory: string }[];
      for (const { release_id, directory } of probes) {
        const cutoff = releaseCutoff(release_id);
        if (!owners.some(o => under(o.directory, directory) && (cutoff === undefined || o.first_generation <= cutoff))) continue;
        db.prepare("INSERT OR IGNORE INTO release_journal.analysis_retired_release VALUES (?,?,?)").run(identity, directory, release_id);
        db.prepare("DELETE FROM analysis_scope_release_probes WHERE release_id=? AND store=?").run(release_id, path);
        db.prepare("DELETE FROM release_journal.analysis_journal_target WHERE release_id=? AND path=? AND probe=1").run(release_id, path);
      }
    }
    const retained = ["analysis_retired_directory", "analysis_named_store", "analysis_named_acquisition"].some(table =>
      (table === "analysis_retired_directory" || present.has(table)) && db.prepare(`SELECT 1 FROM routing.${table} WHERE identity=? LIMIT 1`).get(identity));
    if (!retained && !db.prepare("SELECT 1 FROM release_journal.analysis_retired_release WHERE identity=? LIMIT 1").get(identity)) {
      db.prepare("DELETE FROM routing.analysis_retired_route WHERE identity=?").run(identity);
    }
    return paths.length;
  });
}

export function hasRetiredRoute(index: Client, identity: string): boolean {
  return Boolean(index.prepare("SELECT 1 FROM sqlite_schema WHERE name='analysis_retired_route'").get() &&
    index.prepare("SELECT 1 FROM analysis_retired_route WHERE identity=? LIMIT 1").get(identity));
}

export function resumeAnalysisStore(database: string, store: string, identity: string): void {
  changeRoutes(database, db => {
    if (!db.prepare("SELECT 1 FROM routing.analysis_retired_route WHERE identity=? LIMIT 1").get(identity)) return;
    const path = storedPath(store);
    assertContentStoreOwner(path);
    const owners = db.prepare("SELECT directory,first_generation,last_generation FROM routing.analysis_retired_directory WHERE identity=?").all(identity) as Owner[];
    const releases = db.prepare(`SELECT directory,release_id FROM release_journal.analysis_retired_release WHERE identity=?
      ORDER BY CAST(release_id AS INTEGER),release_id`).all(identity) as { directory: string; release_id: string }[];
    for (const { directory, release_id } of releases) {
      loadNativeBinding().deleteAnalysisScope(path, toNamespacedPath(directory), true, release_id, identity, true);
      const cutoff = releaseCutoff(release_id);
      if (cutoff !== undefined) for (const owner of owners) {
        if (under(owner.directory, directory)) owner.first_generation = Math.max(owner.first_generation, cutoff + 1);
      }
    }
    for (const owner of owners.filter(o => o.first_generation <= o.last_generation)) {
      db.prepare("INSERT OR IGNORE INTO routing.analysis_directory_store VALUES (?,?)").run(owner.directory, path);
      db.prepare(`INSERT INTO routing.analysis_directory_store_identity VALUES (?,?,?,?,?) ON CONFLICT(directory,path,identity)
        DO UPDATE SET first_generation=min(first_generation,excluded.first_generation),last_generation=max(last_generation,excluded.last_generation)`)
        .run(owner.directory, path, identity, owner.first_generation, owner.last_generation);
    }
    const present = tables(db, "routing");
    if (present.has("analysis_named_store")) {
      if (!db.prepare("SELECT 1 FROM pragma_table_info('analysis_named_store','routing') WHERE name='published'").get()) {
        db.exec("ALTER TABLE routing.analysis_named_store ADD COLUMN published INTEGER NOT NULL DEFAULT 0");
      }
      db.prepare(`INSERT INTO routing.analysis_named_store(name,path,identity,published) SELECT name,?,identity,max(published)
        FROM routing.analysis_named_store WHERE identity=? AND path!=? GROUP BY name,identity
        ON CONFLICT(name,path,identity) DO UPDATE SET published=max(published,excluded.published)`).run(path, identity, path);
      db.prepare("DELETE FROM routing.analysis_named_store WHERE identity=? AND path!=?").run(identity, path);
    }
    if (present.has("analysis_named_acquisition")) db.prepare("UPDATE routing.analysis_named_acquisition SET path=? WHERE identity=?").run(path, identity);
    db.prepare("DELETE FROM release_journal.analysis_retired_release WHERE identity=?").run(identity);
    db.prepare("DELETE FROM routing.analysis_retired_directory WHERE identity=?").run(identity);
    db.prepare("DELETE FROM routing.analysis_retired_route WHERE identity=?").run(identity);
    db.prepare("INSERT OR IGNORE INTO routing.analysis_maintenance_store VALUES (?)").run(path);
    db.prepare("INSERT OR IGNORE INTO routing.analysis_maintenance_identity VALUES (?,?)").run(path, identity);
  });
}
