import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { Readable } from "node:stream";
import { afterEach } from "vitest";
import type { AdapterDelivery, AdapterEvent, RerailEventLoop } from "../src/adapter-contract.js";
import { serveHook, type HookInput, type HookOptions, type HookOutput } from "../src/hook-host.js";
import type { SemanticEmbedder } from "../src/semantic-types.js";
import { startTestRepository } from "./support/started-repository.js";

/** Replaces the native model so hooks and runtimes run where the addon is absent. */
export const testEmbedder: SemanticEmbedder = {
  fingerprint: () => ({ modelId: "test", revision: "1", dimensions: 2, weightsSha256: "test", tokenizerSha256: "test", kernelFormatVersion: 1 }),
  embed: async texts => texts.map(() => new Float32Array([1, 0])),
};

export function hookOptions(root: string): HookOptions {
  return { storageRoot: join(root, "sessions"), databasePath: join(root, "rerail.sqlite"), embedder: testEmbedder };
}

/** Per-test scratch directories, each its own RERAIL_HOME and, unless asked otherwise, a started repository. */
export function temporaryDirectories(prefix: string): (options?: { started?: boolean }) => Promise<string> {
  const paths: string[] = [];
  const originalRerailHome = process.env.RERAIL_HOME;
  afterEach(async () => {
    if (originalRerailHome === undefined) delete process.env.RERAIL_HOME;
    else process.env.RERAIL_HOME = originalRerailHome;
    await Promise.all(paths.splice(0).map(path => rm(path, { recursive: true, force: true })));
  });
  return async ({ started = true } = {}) => {
    const path = await mkdtemp(join(tmpdir(), prefix));
    process.env.RERAIL_HOME = join(path, "home");
    paths.push(path);
    if (started) startTestRepository(path, hookOptions(path).databasePath);
    return path;
  };
}

/** Rows from a hook's SQLite session database through a separate read-only connection. */
export function sessionDatabaseQuery<Row>(root: string, sql: string): Row[] {
  const database = new Database(join(root, "rerail.sqlite"), { readonly: true });
  try {
    return database.prepare<[], Row>(sql).all();
  } finally {
    database.close();
  }
}

/** Event ids and kinds a hook lifecycle persisted, in sequence. */
export function eventRows(root: string, kinds: readonly string[]): Array<[string, string]> {
  const kindList = kinds.map(kind => `'${kind}'`).join(", ");
  const sql = `SELECT event_id AS eventId, kind FROM events WHERE kind IN (${kindList}) ORDER BY sequence`;
  return sessionDatabaseQuery<{ eventId: string; kind: string }>(root, sql).map(row => [row.eventId, row.kind]);
}

export function scriptedLoop(script: Readonly<Record<string, readonly AdapterDelivery[]>>): RerailEventLoop & { events: AdapterEvent[] } {
  const events: AdapterEvent[] = [];
  return {
    events,
    async ingest(event) {
      events.push(event);
      return script[event.kind] ?? [];
    },
  };
}

/** Serves one stdin payload through the harness entry and captures both answer streams. */
export async function serveText(run: (input: HookInput) => Promise<HookOutput>, stdin: string): Promise<{ stdout: string; stderr: string }> {
  const written = { stdout: "", stderr: "" };
  await serveHook(run, {
    stdin: Readable.from([stdin]),
    stdout: { write: (chunk, callback) => { written.stdout += chunk; callback(); } },
    stderr: { write: chunk => (written.stderr += chunk) },
  });
  return written;
}
