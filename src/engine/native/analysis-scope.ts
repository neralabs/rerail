import { dirname, join } from "node:path";
import { createHash, randomUUID } from "node:crypto";
import { existsSync } from "node:fs";
import { and, eq, lte, sql } from "drizzle-orm";
import { analysisScopeGeneration, analysisScopeReleaseProbes, analysisScopeReleases } from "../storage/schema.js";
import { openRerailDatabase, openSqliteDatabase, type RerailDatabase } from "../storage/database.js";
import { loadNativeBinding } from "./native.js";
import { contentStoreOverride, contentStorePath } from "./rerail-build.js";
import { assertContentStoreOwner } from "./main-instance.js";
import { storedPath, withinCheckout } from "../catalog/checkout-control.js";

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

export function readAnalysisScopeGeneration(databasePath: string): number {
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

/** Open the store before registering it. Register it before reading the generation. */
export async function acquireDirectoryAnalysisGeneration(databasePath: string, directory: string, store: string): Promise<number> {
  await loadNativeBinding().prepareAnalysisStoreAsync(store);
  registerDirectoryAnalysisStore(databasePath, directory, store);
  return readAnalysisScopeGeneration(databasePath);
}

function registerDirectoryAnalysisStore(databasePath: string, directory: string, store: string): void {
  const index = openSqliteDatabase(storeRegistryPath(databasePath));
  try {
    index.$client.exec("CREATE TABLE IF NOT EXISTS analysis_directory_store (directory TEXT NOT NULL, path TEXT NOT NULL, PRIMARY KEY(directory,path)) STRICT");
    index.$client.prepare("INSERT OR IGNORE INTO analysis_directory_store VALUES (?,?)").run(storedPath(directory), storedPath(store));
  } finally {
    index.$client.close();
  }
}

function analysisStores(databasePath: string): { fallback: Set<string>; registered: { directory: string; path: string }[] } {
  const fallback = new Set([defaultScanStore(databasePath), contentStorePath()]
    .filter(path => existsSync(path)).map(path => storedPath(path)));
  const configured = contentStoreOverride();
  if (configured !== undefined) fallback.add(storedPath(configured));
  const registered: { directory: string; path: string }[] = [];
  const indexPath = storeRegistryPath(databasePath);
  if (!existsSync(indexPath)) return { fallback, registered };
  const index = openSqliteDatabase(indexPath, { readonly: true });
  try {
    const tables = index.$client.prepare("SELECT name FROM sqlite_schema WHERE name IN ('analysis_store', 'analysis_directory_store')").pluck().all();
    if (tables.includes("analysis_store")) {
      for (const path of index.$client.prepare("SELECT path FROM analysis_store").pluck().all() as string[]) fallback.add(path);
    }
    if (tables.includes("analysis_directory_store")) {
      for (const row of index.$client.prepare("SELECT directory,path FROM analysis_directory_store").all() as typeof registered) {
        registered.push(row);
      }
    }
  } finally {
    index.$client.close();
  }
  return { fallback, registered };
}

type Transaction = Parameters<Parameters<RerailDatabase["transaction"]>[0]>[0];

/** Advance the acquisition generation in the same transaction as the stop. */
export function queueAnalysisScopeReleases(tx: Transaction, directories: readonly string[], databasePath: string): void {
  if (directories.length === 0) return;
  const { generation } = tx.insert(analysisScopeGeneration).values({ singleton: 1, generation: INITIAL_GENERATION + 1 })
    .onConflictDoUpdate({ target: analysisScopeGeneration.singleton, set: { generation: sql`${analysisScopeGeneration.generation} + 1` } })
    .returning({ generation: analysisScopeGeneration.generation }).get();
  const home = analysisScopeHome(databasePath);
  const nextAttemptAt = Date.now();
  for (const directory of directories) {
    tx.insert(analysisScopeReleases).values({ id: `${generation - 1}:${randomUUID()}:${home}`, directory, nextAttemptAt }).run();
  }
}

export async function collectAnalysisCaches(databasePath: string): Promise<void> {
  const failures: string[] = [];
  const { fallback, registered } = analysisStores(databasePath);
  for (const store of new Set([...fallback, ...registered.map(row => row.path)])) {
    if (!existsSync(store)) continue;
    try {
      assertContentStoreOwner(store);
      await loadNativeBinding().collectAnalysisCacheAsync(store);
    } catch (error) {
      failures.push(`${store}: ${error instanceof Error ? error.message : String(error)}`);
    }
  }
  if (failures.length > 0) throw new Error(failures.join("\n"));
}

function settleAnalysisScopeReleaseProbes(database: RerailDatabase, home: string, limit: number): void {
  const binding = loadNativeBinding();
  const probes = database.select().from(analysisScopeReleaseProbes)
    .where(limit < 0 ? undefined : lte(analysisScopeReleaseProbes.nextAttemptAt, Date.now()))
    .orderBy(analysisScopeReleaseProbes.nextAttemptAt, analysisScopeReleaseProbes.releaseId, analysisScopeReleaseProbes.store)
    .limit(limit).all();
  for (const { releaseId, directory, store } of probes) {
    const row = and(eq(analysisScopeReleaseProbes.releaseId, releaseId), eq(analysisScopeReleaseProbes.store, store));
    try {
      assertContentStoreOwner(store);
      if (!existsSync(store)) throw new Error("The legacy store is unavailable");
      if (binding.existingDirectoryAnalysisOwnership(store, directory, home)) {
        registerDirectoryAnalysisStore(database.$client.name, directory, store);
        binding.deleteAnalysisScope(store, directory, true, releaseId);
      }
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
    const { fallback, registered } = analysisStores(databasePath);
    const binding = loadNativeBinding();
    const failed = new Map<string, unknown>();
    for (const { id, directory } of pending) {
      const root = storedPath(directory);
      const routes = new Set(registered.filter(row => withinCheckout(root, row.directory)).map(row => row.path));
      let acknowledged = true;
      for (const store of new Set([...routes, ...fallback])) {
        let deliveryTarget = routes.has(store);
        try {
          assertContentStoreOwner(store);
          if (!deliveryTarget) {
            if (!existsSync(store)) throw new Error("The legacy store is unavailable");
            if (!binding.existingDirectoryAnalysisOwnership(store, root, home)) continue;
            deliveryTarget = true;
            registerDirectoryAnalysisStore(databasePath, root, store);
            registered.push({ directory: root, path: store });
          }
          if (failed.has(store)) {
            acknowledged = false;
            continue;
          }
          if (!existsSync(store)) throw new Error("The registered store is unavailable");
          binding.deleteAnalysisScope(store, directory, true, id);
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
