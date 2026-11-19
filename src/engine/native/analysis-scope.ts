import { dirname, join, sep, toNamespacedPath } from "node:path";
import { createHash, randomUUID } from "node:crypto";
import { existsSync, statSync } from "node:fs";
import { and, eq, lte } from "drizzle-orm";
import { analysisScopeGeneration, analysisScopeReleaseProbes, analysisScopeReleases } from "../storage/schema.js";
import { openRerailDatabase, openSqliteDatabase, type RerailDatabase } from "../storage/database.js";
import { loadNativeBinding, type NativeAnalysisScopeRun } from "./native.js";
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

function maintenanceRegistration(index: RerailDatabase) {
  index.$client.exec("CREATE TABLE IF NOT EXISTS analysis_maintenance_store (path TEXT PRIMARY KEY) STRICT");
  index.$client.exec("CREATE TABLE IF NOT EXISTS analysis_maintenance_identity (path TEXT NOT NULL, identity TEXT NOT NULL, PRIMARY KEY(path,identity)) STRICT");
  const remember = index.$client.prepare("INSERT OR IGNORE INTO analysis_maintenance_store VALUES (?)");
  const identify = index.$client.prepare("INSERT OR IGNORE INTO analysis_maintenance_identity VALUES (?,?)");
  return (path: string, identity: string | null) => {
    remember.run(path);
    if (identity) identify.run(path, identity);
  };
}

export function acquireNamedAnalysisRun(databasePath: string, name: string, store: string, scanId: string) {
  const index = openSqliteDatabase(storeRegistryPath(databasePath));
  let run: NativeAnalysisScopeRun | undefined;
  try {
    return index.$client.transaction(() => {
      ensureNamedRoutes(index);
      run = new (loadNativeBinding().AnalysisScopeRun)(store, name, false, scanId, 0);
      const path = storedPath(store);
      maintenanceRegistration(index)(path, run.storeIdentity);
      index.$client.prepare("INSERT INTO analysis_named_acquisition(name,path,identity,acquisition,scan) VALUES (?,?,?,?,?)")
        .run(name, path, run.storeIdentity, run.acquisition, scanId);
      return run;
    }).immediate();
  } catch (error) {
    run?.finish(false);
    throw error;
  } finally { index.$client.close(); }
}

function ensureNamedRoutes(index: RerailDatabase): void {
  index.$client.exec(`CREATE TABLE IF NOT EXISTS analysis_named_store (
    name TEXT NOT NULL, path TEXT NOT NULL, identity TEXT NOT NULL, published INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(name,path,identity)) STRICT;
    CREATE TABLE IF NOT EXISTS analysis_named_acquisition (
    name TEXT NOT NULL, path TEXT NOT NULL, identity TEXT NOT NULL, acquisition TEXT NOT NULL, scan TEXT NOT NULL,
    PRIMARY KEY(name,identity,acquisition,scan)) STRICT;
    CREATE INDEX IF NOT EXISTS analysis_named_acquisition_by_path ON analysis_named_acquisition(path)`);
  if (!index.$client.prepare("SELECT 1 FROM pragma_table_info('analysis_named_store') WHERE name='published'").get()) {
    index.$client.exec("ALTER TABLE analysis_named_store ADD COLUMN published INTEGER NOT NULL DEFAULT 0");
  }
}

function finishNamedRoute(index: RerailDatabase, name: string, identity: string, acquisition: string, scan: string, published: boolean): void {
  const key = [name, identity, acquisition, scan];
  if (published) index.$client.prepare(`INSERT INTO analysis_named_store(name,path,identity,published)
    SELECT name,path,identity,1 FROM analysis_named_acquisition WHERE name=? AND identity=? AND acquisition=? AND scan=?
    ON CONFLICT(name,path,identity) DO UPDATE SET published=1`).run(...key);
  index.$client.prepare("DELETE FROM analysis_named_acquisition WHERE name=? AND identity=? AND acquisition=? AND scan=?").run(...key);
}

export function finishNamedAnalysisRegistration(databasePath: string, name: string, scan: string, run: NativeAnalysisScopeRun): void {
  if (run.publication == null) return;
  const index = openSqliteDatabase(storeRegistryPath(databasePath));
  try {
    index.$client.transaction(() => finishNamedRoute(index, name, run.storeIdentity, run.acquisition, scan, run.publication === "published")).immediate();
  } finally { index.$client.close(); }
}

/** A crashed caller can leave a route on either side of publication. Consult the store that issued its acquisition. */
function recoverNamedAcquisitions(index: RerailDatabase, column: "name" | "path", value: string): void {
  const pending = index.$client.prepare(`SELECT name,path,identity,acquisition,scan FROM analysis_named_acquisition WHERE ${column}=?`)
    .all(value) as { name: string; path: string; identity: string; acquisition: string; scan: string }[];
  for (const { name, path, identity, acquisition, scan } of pending) {
    let state;
    try { state = loadNativeBinding().namedAnalysisAcquisitionState(path, name, identity, acquisition, scan); }
    catch { continue; } // An unavailable store can still own the run.
    if (state !== "pending") finishNamedRoute(index, name, identity, acquisition, scan, state === "published");
  }
}

function recoverNamedRoutes(index: RerailDatabase, name: string, requested?: string): void {
  recoverNamedAcquisitions(index, "name", name);
  const legacy = index.$client.prepare("SELECT path,identity FROM analysis_named_store WHERE name=? AND published=0")
    .all(name) as { path: string; identity: string }[];
  for (const { path, identity } of legacy) {
    // Keep the original identity guard for an explicit release, even for a store with zero owners.
    if (path === requested) continue;
    let state;
    try { state = loadNativeBinding().legacyNamedAnalysisRouteState(path, name, identity); }
    catch { continue; }
    if (state === "published") index.$client.prepare("UPDATE analysis_named_store SET published=1 WHERE name=? AND path=? AND identity=?").run(name, path, identity);
    if (state === "absent") index.$client.prepare("DELETE FROM analysis_named_store WHERE name=? AND path=? AND identity=?").run(name, path, identity);
  }
}

export async function deleteNamedAnalysisScope(databasePath: string, name: string, requested?: string): Promise<{ store: string; hadMembers: boolean }> {
  const index = openSqliteDatabase(storeRegistryPath(databasePath));
  const binding = loadNativeBinding();
  let released;
  try {
    // Acquisition and release take the registry's writer before touching the content store.
    released = index.$client.transaction(() => {
      ensureNamedRoutes(index);
      recoverNamedRoutes(index, name, requested === undefined ? undefined : storedPath(requested));
      const routes = index.$client.prepare(`SELECT path,identity FROM analysis_named_store WHERE name=?
        UNION SELECT path,identity FROM analysis_named_acquisition WHERE name=? ORDER BY path,identity`)
        .all(name, name) as { path: string; identity: string }[];
      const paths = [...new Set(routes.map(route => route.path))];
      if (requested === undefined && paths.length > 1) {
        throw new Error(`Scope ${JSON.stringify(name)} has multiple stores. Select one with --store <file>.\n${paths.join("\n")}`);
      }
      const store = requested === undefined ? paths[0] ?? storedPath(scanContentStore(databasePath)) : storedPath(requested);
      assertContentStoreOwner(store);
      const identities = routes.filter(route => route.path === store).map(route => route.identity);
      const result = binding.releaseNamedAnalysisScope(store, name, identities);
      if (result) {
        index.$client.prepare("DELETE FROM analysis_named_store WHERE name=? AND path=? AND identity=?").run(name, store, result.identity);
        index.$client.prepare("DELETE FROM analysis_named_acquisition WHERE name=? AND path=? AND identity=?").run(name, store, result.identity);
      }
      return { store, result };
    }).immediate();
  } finally { index.$client.close(); }
  if (released.result) {
    try { await binding.collectAnalysisCacheAsync(released.store, [released.result.identity]); }
    catch (error) { throw new Error(`Scope release completed in ${released.store}. Cache collection failed. ${String(error)}`); }
  }
  return { store: released.store, hadMembers: released.result?.hadMembers ?? false };
}

function registerDirectoryAnalysisStore(databasePath: string, root: string, store: string, identity: string, floor?: number): number {
  const index = openSqliteDatabase(storeRegistryPath(databasePath));
  try {
    return index.$client.transaction(() => {
      index.$client.exec("CREATE TABLE IF NOT EXISTS analysis_directory_store (directory TEXT NOT NULL, path TEXT NOT NULL, PRIMARY KEY(directory,path)) STRICT");
      index.$client.exec(`CREATE TABLE IF NOT EXISTS analysis_directory_store_identity (
        directory TEXT NOT NULL, path TEXT NOT NULL, identity TEXT NOT NULL, first_generation INTEGER NOT NULL, last_generation INTEGER NOT NULL,
        PRIMARY KEY(directory,path,identity), FOREIGN KEY(directory,path) REFERENCES analysis_directory_store(directory,path) ON DELETE CASCADE ON UPDATE CASCADE) STRICT`);
      const path = storedPath(store);
      index.$client.prepare("INSERT OR IGNORE INTO analysis_directory_store VALUES (?,?)").run(root, path);
      maintenanceRegistration(index)(path, identity);
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

function analysisStores(databasePath: string, root?: string, cutoff = Number.MAX_SAFE_INTEGER) {
  const fallback = new Set([defaultScanStore(databasePath), contentStorePath()]
    .filter(path => existsSync(path)).map(path => storedPath(path)));
  const configured = contentStoreOverride();
  if (configured !== undefined) fallback.add(storedPath(configured));
  const registered: StoreRoute[] = [];
  const maintenance = new Map<string, string[]>();
  const indexPath = storeRegistryPath(databasePath);
  if (!existsSync(indexPath)) return { fallback, registered, maintenance };
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
      const sources = ["analysis_maintenance_identity", "analysis_directory_store_identity", "analysis_named_store"].filter(name => tables.includes(name));
      if (sources.length > 0) {
        const rows = index.$client.prepare(sources.map(name => "SELECT path,identity FROM " + name).join(" UNION ")).iterate();
        for (const { path, identity } of rows as Iterable<{ path: string; identity: string }>) {
          if (!identity) continue;
          const identities = maintenance.get(path);
          if (identities) identities.push(identity);
          else maintenance.set(path, [identity]);
        }
      }
    }
  } finally {
    index.$client.close();
  }
  return { fallback, registered, maintenance };
}

/** Advance only routes this delivery covered. A concurrent new acquisition keeps its registration. */
function acknowledgeRoutes(databasePath: string, routes: StoreRoute[], cutoff: number | undefined): void {
  if (cutoff === undefined || routes.length === 0) return;
  const index = openSqliteDatabase(storeRegistryPath(databasePath));
  try {
    index.$client.transaction(() => {
      const identified = index.$client.prepare("SELECT 1 FROM sqlite_schema WHERE name='analysis_directory_store_identity'").get();
      const remember = maintenanceRegistration(index);
      for (const { directory, path, identity } of routes) {
        remember(path, identity);
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
  const collected: string[] = [];
  const { fallback: stores, maintenance } = analysisStores(databasePath);
  for (const store of stores) {
    try {
      if (statSync(store, { throwIfNoEntry: false }) === undefined) continue;
      assertContentStoreOwner(store);
      await loadNativeBinding().collectAnalysisCacheAsync(store, maintenance.get(store));
      collected.push(store);
    } catch (error) {
      failures.push(`${store}: ${error instanceof Error ? error.message : String(error)}`);
    }
  }
  const registry = storeRegistryPath(databasePath);
  if (collected.length && existsSync(registry)) {
    const index = openSqliteDatabase(registry);
    try {
      if (index.$client.prepare("SELECT 1 FROM sqlite_schema WHERE name='analysis_named_acquisition'").get()) {
        index.$client.transaction(() => {
          for (const path of collected) recoverNamedAcquisitions(index, "path", path);
        }).immediate();
      }
    } finally { index.$client.close(); }
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
