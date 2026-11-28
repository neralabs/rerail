import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { Readable } from "node:stream";
import { afterEach } from "vitest";
import type { AdapterDelivery, AdapterEvent, EventAnswerFacts, RerailEventLoop, SemanticEmbedder } from "../../src/engine/index.js";
import { serveHook, type HookInput, type HookOptions, type HookOutput } from "../../src/harness/index.js";
import { startTestRepository } from "./started-repository.js";
import { removeFolder } from "./platform/remove-folder.js";
import { testEngineEndpoint } from "./platform/engine-endpoint.js";

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
    await Promise.all(paths.splice(0).map(async path => await removeFolder(path)));
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

/** `loop`, whose answers carry `facts` beside their deliveries, as a daemon's that knows them answers a hook. */
export function answering<Loop extends RerailEventLoop>(loop: Loop, facts: EventAnswerFacts): Loop {
  return Object.assign(loop, { answerFacts: () => facts });
}

/** A daemon a hook may read from: its loopback dashboard origin and the paths of the requests it has been sent. */
export interface CountingDaemon {
  readonly origin: string;
  readonly requests: string[];
  close(): Promise<void>;
}

/**
 * Records in `home`'s daemon.json a live daemon (this process) whose dashboard is a loopback listener that counts what
 * it is asked and answers every request 503, so a hook's reads find nothing beyond the bare origin.
 */
export async function countingDaemon(home: string): Promise<CountingDaemon> {
  const requests: string[] = [];
  const server = createServer((request, response) => {
    requests.push(new URL(request.url ?? "/", "http://127.0.0.1/").pathname);
    response.writeHead(503).end();
  });
  await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
  const origin = `http://127.0.0.1:${(server.address() as AddressInfo).port}/`;
  await mkdir(home, { recursive: true });
  await writeFile(join(home, "daemon.json"), JSON.stringify({ pid: process.pid, host: "127.0.0.1", port: (server.address() as AddressInfo).port, url: origin,
    version: "test", databasePath: join(home, "rerail.sqlite"), kernelSocketPath: testEngineEndpoint(home), ui: "bundled", startedAt: 1, packageRoot: home }));
  const close = async (): Promise<void> => {
    server.closeAllConnections();
    await new Promise(resolve => server.close(resolve));
  };
  return { origin, requests, close };
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
