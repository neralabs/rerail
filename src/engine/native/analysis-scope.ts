import { dirname, join, sep, toNamespacedPath } from "node:path";
import { createHash, randomUUID } from "node:crypto";
import { existsSync, statSync } from "node:fs";
import { and, eq, lte } from "drizzle-orm";
import { analysisScopeGeneration, analysisScopeReleaseProbes, analysisScopeReleases } from "../storage/schema.js";
import { openRerailDatabase, openSqliteDatabase, type RerailDatabase } from "../storage/database.js";
import { loadNativeBinding } from "./native.js";
import { contentStoreOverride, contentStorePath } from "./rerail-build.js";
import { assertContentStoreOwner } from "./main-instance.js";
import { storedPath } from "../catalog/checkout-control.js";

export function scanContentStore(databasePath: string): string {
  return contentStoreOverride() ?? defaultScanStore(databasePath);
}

function defaultScanStore(databasePath: string): string {
  return join(dirname(databasePath), `revision-${loadNativeBinding().contentStoreFileName()}`);
}

// Keep generation tokens above the legacy microsecond timestamps already stored on ownership rows.
const INITIAL_GENERATION = 2 ** 52;
const RELEASE_RETRY_DELAY_MS = 5_000;

export function analysisScopeHome(databasePath: string): string {
  return createHash("sha256").update(storedPath(databasePath)).digest("hex");
}

function readControlGeneration(databasePath: string): number {
  if (!existsSync(databasePath)) return INITIAL_GENERATION;
  const reader = openSqliteDatabase(databasePath, { readonly: true });
  try {
    if (!reader.$client.prepare("SELECT 1 FROM sqlite_schema WHERE name = 'analysis_scope_generation'").get()) return INITIAL_GENERATION;
    return reader.select().from(analysisScopeGeneration).get()?.generation ?? INITIAL_GENERATION;
  } finally {
    reader.$client.close();
  }
}

function storeRegistryPath(databasePath: string): string {
  return `${storedPath(databasePath)}.analysis-stores.sqlite`;
}

export function readAnalysisScopeGeneration(databasePath: string): number {
  const current = readControlGeneration(databasePath);
  const path = storeRegistryPath(databasePath);
  if (!existsSync(path)) return current;
  const index = openSqliteDatabase(path, { readonly: true });
  try {
    if (!index.$client.prepare("SELECT 1 FROM sqlite_schema WHERE name='analysis_registry_generation'").get()) return current;
    const generation = index.$client.prepare("SELECT generation FROM analysis_registry_generation WHERE singleton=1").pluck().get() as number | undefined;
    return Math.max(current, generation ?? INITIAL_GENERATION);
  } finally { index.$client.close(); }
}

export interface DirectoryAnalysisRegistration { generation: number; identity: string }

export async function acquireDirectoryAnalysisGeneration(databasePath: string, directory: string, store: string): Promise<number> {
  return (await acquireDirectoryAnalysisRegistration(databasePath, directory, store)).generation;
}

/** Register the observed store and obtain its generation in one registry transaction. */
export async function acquireDirectoryAnalysisRegistration(databasePath: string, directory: string, store: string): Promise<DirectoryAnalysisRegistration> {
  const root = storedPath(directory);
  const { cutoff, identity } = await loadNativeBinding().prepareAnalysisStoreRegistrationAsync(store, root, analysisScopeHome(databasePath));
  const floor = Math.max(readControlGeneration(databasePath), (cutoff ?? 0) + 1);
  return { generation: registerDirectoryAnalysisStore(databasePath, root, store, identity, floor), identity };
}

function recordRegistryGeneration(index: RerailDatabase, floor: number, increment: 0 | 1): number {
  index.$client.exec("CREATE TABLE IF NOT EXISTS analysis_registry_generation (singleton INTEGER PRIMARY KEY CHECK(singleton=1), generation INTEGER NOT NULL) STRICT");
  const generation = index.$client.prepare(`INSERT INTO analysis_registry_generation VALUES (1,?+?)
    ON CONFLICT(singleton) DO UPDATE SET generation=max(generation,?)+? RETURNING generation`)
    .pluck().get(floor, increment, floor, increment) as number;
  if (!Number.isSafeInteger(generation)) throw new Error("Analysis scope generation exceeds the supported integer range");
  return generation;
}

interface StoreRoute { directory: string; path: string; identity: string | null }

function registerDirectoryAnalysisStore(databasePath: string, root: string, store: string, identity: string, floor?: number): number {
  const index = openSqliteDatabase(storeRegistryPath(databasePath));
  try {
    return index.$client.transaction(() => {
      index.$client.exec("CREATE TABLE IF NOT EXISTS analysis_directory_store (directory TEXT NOT NULL, path TEXT NOT NULL, PRIMARY KEY(directory,path)) STRICT");
      index.$client.exec(`CREATE TABLE IF NOT EXISTS analysis_directory_store_identity (
        directory TEXT NOT NULL, path TEXT NOT NULL, identity TEXT NOT NULL, first_generation INTEGER NOT NULL, last_generation INTEGER NOT NULL,
        PRIMARY KEY(directory,path,identity), FOREIGN KEY(directory,path) REFERENCES analysis_directory_store(directory,path) ON DELETE CASCADE ON UPDATE CASCADE) STRICT`);
      index.$client.exec("CREATE TABLE IF NOT EXISTS analysis_maintenance_store (path TEXT PRIMARY KEY) STRICT");
      const path = storedPath(store);
      index.$client.prepare("INSERT OR IGNORE INTO analysis_directory_store VALUES (?,?)").run(root, path);
      index.$client.prepare("INSERT OR IGNORE INTO analysis_maintenance_store VALUES (?)").run(path);
      const generation = floor === undefined ? 0 : recordRegistryGeneration(index, floor, 0);
      index.$client.prepare(`INSERT INTO analysis_directory_store_identity VALUES (?,?,?,?,?)
        ON CONFLICT(directory,path,identity) DO UPDATE SET first_generation=min(first_generation,excluded.first_generation),
          last_generation=max(last_generation,excluded.last_generation)`).run(root, path, identity, generation, generation);
      return generation;
    }).immediate();
  } finally {
    index.$client.close();
  }
}

function analysisStores(databasePath: string, root?: string, cutoff = Number.MAX_SAFE_INTEGER): { fallback: Set<string>; registered: StoreRoute[] } {
  const fallback = new Set([defaultScanStore(databasePath), contentStorePath()]
    .filter(path => existsSync(path)).map(path => storedPath(path)));
  const configured = contentStoreOverride();
  if (configured !== undefined) fallback.add(storedPath(configured));
  const registered: StoreRoute[] = [];
  const indexPath = storeRegistryPath(databasePath);
  if (!existsSync(indexPath)) return { fallback, registered };
  const index = openSqliteDatabase(indexPath, { readonly: true });
  try {
    const tables = index.$client.prepare("SELECT name FROM sqlite_schema WHERE type='table'").pluck().all();
    if (tables.includes("analysis_store")) {
      for (const path of index.$client.prepare("SELECT path FROM analysis_store").pluck().all() as string[]) fallback.add(path);
    }
    if (root === undefined && tables.includes("analysis_directory_store")) {
      for (const path of index.$client.prepare("SELECT DISTINCT path FROM analysis_directory_store").pluck().iterate()) fallback.add(path as string);
    } else if (root !== undefined && tables.includes("analysis_directory_store")) {
      const identified = tables.includes("analysis_directory_store_identity");
      const query = identified ? "SELECT d.directory,d.path,i.identity FROM analysis_directory_store d LEFT JOIN analysis_directory_store_identity i USING(directory,path)"
        : "SELECT d.directory,d.path,NULL AS identity FROM analysis_directory_store d";
      const prefix = root.endsWith(sep) ? root : root + sep;
      // All descendants share this byte prefix. The next character after the separator is its exclusive upper bound.
      const end = prefix.slice(0, -1) + String.fromCharCode(sep.charCodeAt(0) + 1);
      const conditions = " WHERE (d.directory=? OR (d.directory>=? AND d.directory<?))"
        + (identified ? " AND (i.first_generation IS NULL OR i.first_generation<=?)" : "");
      const args = [root, prefix, end, ...(identified ? [cutoff] : [])];
      for (const row of index.$client.prepare(query + conditions).iterate(...args)) registered.push(row as StoreRoute);
    }
    if (root === undefined && tables.includes("analysis_maintenance_store")) {
      for (const path of index.$client.prepare("SELECT path FROM analysis_maintenance_store").pluck().all() as string[]) fallback.add(path);
    }
  } finally {
    index.$client.close();
  }
  return { fallback, registered };
}

/** Advance only routes this delivery covered. A concurrent new acquisition keeps its registration. */
function acknowledgeRoutes(databasePath: string, routes: StoreRoute[], cutoff: number | undefined): void {
  if (cutoff === undefined || routes.length === 0) return;
  const index = openSqliteDatabase(storeRegistryPath(databasePath));
  try {
    index.$client.transaction(() => {
      const identified = index.$client.prepare("SELECT 1 FROM sqlite_schema WHERE name='analysis_directory_store_identity'").get();
      index.$client.exec("CREATE TABLE IF NOT EXISTS analysis_maintenance_store (path TEXT PRIMARY KEY) STRICT");
      for (const { directory, path, identity } of routes) {
        index.$client.prepare("INSERT OR IGNORE INTO analysis_maintenance_store VALUES (?)").run(path);
        if (identified) {
          index.$client.prepare("DELETE FROM analysis_directory_store_identity WHERE directory=? AND path=? AND identity=? AND last_generation<=?").run(directory, path, identity, cutoff);
          index.$client.prepare("UPDATE analysis_directory_store_identity SET first_generation=? WHERE directory=? AND path=? AND identity=? AND first_generation<=?")
            .run(cutoff + 1, directory, path, identity, cutoff);
        }
        const live = identified ? " AND NOT EXISTS (SELECT 1 FROM analysis_directory_store_identity i WHERE i.directory=d.directory AND i.path=d.path)" : "";
        index.$client.prepare("DELETE FROM analysis_directory_store AS d WHERE directory=? AND path=?" + live).run(directory, path);
      }
    }).immediate();
  } finally { index.$client.close(); }
}

function releaseCutoff(id: string): number | undefined {
  const colon = id.indexOf(":");
  if (colon < 0) return undefined;
  const prefix = id.slice(0, colon);
  const cutoff = Number(prefix);
  if (!/^\d+$/u.test(prefix) || !Number.isSafeInteger(cutoff)) throw new Error("Invalid analysis scope release cutoff");
  return cutoff;
}

function discoverReleaseRoute(databasePath: string, directory: string, store: string, home: string): StoreRoute[] {
  const identity = loadNativeBinding().owningDirectoryAnalysisStore(store, toNamespacedPath(directory), home, true);
  if (identity == null) return [];
  registerDirectoryAnalysisStore(databasePath, directory, store, identity);
  return [{ directory, path: store, identity }];
}

function deliverRelease(databasePath: string, root: string, store: string, id: string, routes: StoreRoute[]): void {
  let failure: unknown;
  for (const identity of new Set(routes.map(route => route.identity))) {
    try {
      loadNativeBinding().deleteAnalysisScope(store, toNamespacedPath(root), true, id, identity || undefined, true);
      acknowledgeRoutes(databasePath, routes.filter(route => route.identity === identity), releaseCutoff(id));
    } catch (error) { failure = error; }
  }
  if (failure !== undefined) throw failure;
}

type Transaction = Parameters<Parameters<RerailDatabase["transaction"]>[0]>[0];

/** Advance the acquisition generation in the same transaction as the stop. */
export function queueAnalysisScopeReleases(tx: Transaction, directories: readonly string[], databasePath: string): void {
  if (directories.length === 0) return;
  // Serialize stops on the control writer before reserving a generation in the registry.
  tx.insert(analysisScopeGeneration).values({ singleton: 1, generation: INITIAL_GENERATION }).onConflictDoNothing().run();
  const current = tx.select().from(analysisScopeGeneration).get()!.generation;
  const index = openSqliteDatabase(storeRegistryPath(databasePath));
  let generation: number;
  try {
    generation = index.$client.transaction(() => recordRegistryGeneration(index, current, 1)).immediate();
  } finally { index.$client.close(); }
  tx.update(analysisScopeGeneration).set({ generation }).where(eq(analysisScopeGeneration.singleton, 1)).run();
  const home = analysisScopeHome(databasePath);
  const nextAttemptAt = Date.now();
  for (const directory of directories) {
    tx.insert(analysisScopeReleases).values({ id: `${generation - 1}:${randomUUID()}:${home}`, directory: storedPath(directory), nextAttemptAt }).run();
  }
}

export async function collectAnalysisCaches(databasePath: string): Promise<void> {
  const failures: string[] = [];
  const { fallback: stores } = analysisStores(databasePath);
  for (const store of stores) {
    try {
      if (statSync(store, { throwIfNoEntry: false }) === undefined) continue;
      assertContentStoreOwner(store);
      await loadNativeBinding().collectAnalysisCacheAsync(store);
    } catch (error) {
      failures.push(`${store}: ${error instanceof Error ? error.message : String(error)}`);
    }
  }
  if (failures.length > 0) throw new Error(failures.join("\n"));
}

function settleAnalysisScopeReleaseProbes(database: RerailDatabase, home: string, limit: number): void {
  const databasePath = database.$client.name;
  const probes = database.select().from(analysisScopeReleaseProbes)
    .where(limit < 0 ? undefined : lte(analysisScopeReleaseProbes.nextAttemptAt, Date.now()))
    .orderBy(analysisScopeReleaseProbes.nextAttemptAt, analysisScopeReleaseProbes.releaseId, analysisScopeReleaseProbes.store)
    .limit(limit).all();
  for (const { releaseId, directory, store } of probes) {
    const row = and(eq(analysisScopeReleaseProbes.releaseId, releaseId), eq(analysisScopeReleaseProbes.store, store));
    try {
      assertContentStoreOwner(store);
      let routes = analysisStores(databasePath, directory, releaseCutoff(releaseId)).registered.filter(route => route.path === store);
      if (routes.length === 0) routes = discoverReleaseRoute(databasePath, directory, store, home);
      deliverRelease(databasePath, directory, store, releaseId, routes);
    } catch {
      database.update(analysisScopeReleaseProbes).set({ nextAttemptAt: Date.now() + RELEASE_RETRY_DELAY_MS }).where(row).run();
      continue;
    }
    database.delete(analysisScopeReleaseProbes).where(row).run();
  }
}

/** The daemon owns the content store even if the checkout has no active session. */
export function settleAnalysisScopeReleases(databasePath: string, limit = 32): void {
  if (!existsSync(databasePath)) return;
  const reader = openSqliteDatabase(databasePath, { readonly: true });
  const queued = (() => {
    try {
      const tables = reader.$client.prepare("SELECT name FROM sqlite_schema WHERE name IN ('analysis_scope_releases', 'analysis_scope_release_probes')").pluck().all();
      return (tables.includes("analysis_scope_releases") && reader.select({ id: analysisScopeReleases.id }).from(analysisScopeReleases).limit(1).get())
        || (tables.includes("analysis_scope_release_probes") && reader.select({ id: analysisScopeReleaseProbes.releaseId }).from(analysisScopeReleaseProbes).limit(1).get());
    }
    finally { reader.$client.close(); }
  })();
  if (!queued) return;
  const database = openRerailDatabase(databasePath);
  try {
    const home = analysisScopeHome(databasePath);
    settleAnalysisScopeReleaseProbes(database, home, limit);
    const pending = database.select().from(analysisScopeReleases)
      .where(limit < 0 ? undefined : lte(analysisScopeReleases.nextAttemptAt, Date.now()))
      .orderBy(analysisScopeReleases.nextAttemptAt, analysisScopeReleases.id).limit(limit).all();
    if (pending.length === 0) return;
    const failed = new Map<string, unknown>();
    for (const { id, directory: root } of pending) {
      const { fallback, registered } = analysisStores(databasePath, root, releaseCutoff(id));
      const routes = new Set(registered.map(row => row.path));
      let acknowledged = true;
      for (const store of new Set([...routes, ...fallback])) {
        let deliveryTarget = routes.has(store);
        try {
          assertContentStoreOwner(store);
          const targets = deliveryTarget ? registered.filter(row => row.path === store) : discoverReleaseRoute(databasePath, root, store, home);
          deliveryTarget = targets.length > 0;
          deliverRelease(databasePath, root, store, id, targets);
        } catch (error) {
          if (!deliveryTarget) {
            database.insert(analysisScopeReleaseProbes).values({
              releaseId: id, directory: root, store, nextAttemptAt: Date.now() + RELEASE_RETRY_DELAY_MS,
            }).onConflictDoNothing().run();
            continue;
          }
          failed.set(store, error);
          acknowledged = false;
        }
      }
      if (acknowledged) database.delete(analysisScopeReleases).where(eq(analysisScopeReleases.id, id)).run();
      else database.update(analysisScopeReleases).set({ nextAttemptAt: Date.now() + RELEASE_RETRY_DELAY_MS })
        .where(eq(analysisScopeReleases.id, id)).run();
    }
    if (failed.size > 0) {
      const reasons = [...failed].map(([store, error]) => `${store}: ${error instanceof Error ? error.message : String(error)}`);
      throw new Error(`Could not deliver analysis scope releases. ${reasons.join("\n")}`);
    }
  } finally {
    database.$client.close();
  }
}
