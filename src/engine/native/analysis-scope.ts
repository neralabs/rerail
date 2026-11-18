import { dirname, join } from "node:path";
import { createHash, randomUUID } from "node:crypto";
import { existsSync } from "node:fs";
import { eq, sql } from "drizzle-orm";
import { analysisScopeGeneration, analysisScopeReleases } from "../storage/schema.js";
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

/** Register the store before acquiring a generation. A stop can then find that generation at any store it reaches. */
export function acquireDirectoryAnalysisGeneration(databasePath: string, store: string): number {
  const index = openSqliteDatabase(storeRegistryPath(databasePath));
  try {
    index.$client.exec("CREATE TABLE IF NOT EXISTS analysis_store (path TEXT PRIMARY KEY NOT NULL) STRICT");
    index.$client.prepare("INSERT OR IGNORE INTO analysis_store VALUES (?)").run(storedPath(store));
  } finally {
    index.$client.close();
  }
  return readAnalysisScopeGeneration(databasePath);
}

function analysisStores(databasePath: string): Set<string> {
  const stores = new Set([defaultScanStore(databasePath), scanContentStore(databasePath), contentStorePath()]
    .filter(path => existsSync(path)).map(path => storedPath(path)));
  const indexPath = storeRegistryPath(databasePath);
  if (!existsSync(indexPath)) return stores;
  const index = openSqliteDatabase(indexPath, { readonly: true });
  try {
    if (!index.$client.prepare("SELECT 1 FROM sqlite_schema WHERE name = 'analysis_store'").get()) return stores;
    for (const path of index.$client.prepare("SELECT path FROM analysis_store").pluck().all() as string[]) {
      stores.add(path);
    }
  } finally {
    index.$client.close();
  }
  return stores;
}

type Transaction = Parameters<Parameters<RerailDatabase["transaction"]>[0]>[0];

/** Advance the acquisition generation in the same transaction as the stop. */
export function queueAnalysisScopeReleases(tx: Transaction, directories: readonly string[], databasePath: string): void {
  if (directories.length === 0) return;
  const { generation } = tx.insert(analysisScopeGeneration).values({ singleton: 1, generation: INITIAL_GENERATION + 1 })
    .onConflictDoUpdate({ target: analysisScopeGeneration.singleton, set: { generation: sql`${analysisScopeGeneration.generation} + 1` } })
    .returning({ generation: analysisScopeGeneration.generation }).get();
  const home = analysisScopeHome(databasePath);
  for (const directory of directories) {
    tx.insert(analysisScopeReleases).values({ id: `${generation - 1}:${randomUUID()}:${home}`, directory }).run();
  }
}

export async function collectAnalysisCaches(databasePath: string): Promise<void> {
  const failures: string[] = [];
  for (const store of analysisStores(databasePath)) {
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

/** The daemon owns the content store even if the checkout has no active session. */
export function settleAnalysisScopeReleases(databasePath: string, limit = 32): void {
  if (!existsSync(databasePath)) return;
  const reader = openSqliteDatabase(databasePath, { readonly: true });
  const pending = (() => {
    try { return reader.select().from(analysisScopeReleases).limit(limit).all(); }
    finally { reader.$client.close(); }
  })();
  if (pending.length === 0) return;
  const stores = analysisStores(databasePath);
  const database = openRerailDatabase(databasePath);
  try {
    const failed = new Map<string, unknown>();
    for (const { id, directory } of pending) {
      let acknowledged = true;
      for (const store of stores) {
        if (failed.has(store)) {
          acknowledged = false;
          continue;
        }
        try {
          assertContentStoreOwner(store);
          if (!existsSync(store)) throw new Error("The registered store is unavailable");
          loadNativeBinding().deleteAnalysisScope(store, directory, true, id);
        } catch (error) {
          failed.set(store, error);
          acknowledged = false;
        }
      }
      if (acknowledged) database.delete(analysisScopeReleases).where(eq(analysisScopeReleases.id, id)).run();
    }
    if (failed.size > 0) {
      const reasons = [...failed].map(([store, error]) => `${store}: ${error instanceof Error ? error.message : String(error)}`);
      throw new Error(`Could not deliver analysis scope releases. ${reasons.join("\n")}`);
    }
  } finally {
    database.$client.close();
  }
}
