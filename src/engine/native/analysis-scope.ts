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
  return contentStoreOverride() ?? join(dirname(databasePath), `revision-${loadNativeBinding().contentStoreFileName()}`);
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
  const scanned = scanContentStore(databasePath);
  const stores = new Set([scanned, contentStorePath()]);
  for (const store of stores) {
    if (!existsSync(store)) continue;
    assertContentStoreOwner(store);
    await loadNativeBinding().collectAnalysisCacheAsync(store);
  }
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
  const database = openRerailDatabase(databasePath);
  try {
    const stores = new Set([scanContentStore(databasePath), contentStorePath()]);
    for (const { id, directory } of pending) {
      for (const store of stores) {
        assertContentStoreOwner(store);
        loadNativeBinding().deleteAnalysisScope(store, directory, true, id);
      }
      database.delete(analysisScopeReleases).where(eq(analysisScopeReleases.id, id)).run();
    }
  } finally {
    database.$client.close();
  }
}
