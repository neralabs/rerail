import { readFileSync } from "node:fs";
import { mkdir, mkdtemp, readdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { engineLink, startEngineService, type EngineService } from "../../../src/runtime/engine-service.js";
import { keepReceipt, SqliteSessionStore } from "../../../src/runtime/session-store.js";
import { testEmbedder } from "../../support/hook-events.js";
import { startTestRepository } from "../../support/started-repository.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

// Every read goes to the real one unless a case makes a read fail.
vi.mock("node:fs", { spy: true });
const real = await vi.importActual<typeof import("node:fs")>("node:fs");

const roots: string[] = [];
const services: EngineService[] = [];
afterEach(async () => {
  vi.useRealTimers();
  vi.mocked(readFileSync).mockReset();
  await Promise.all(services.splice(0).map(service => service.close()));
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

/**
 * An engine service over one started checkout, with the capability `accessToken` when one is given; `beforeStart` runs
 * once the checkout is started and the database made, before the service starts. `startAgain` closes the service and
 * starts the next one over the same database, as a daemon's next start does.
 */
async function serviceOverCheckout(accessToken?: string, beforeStart?: (databasePath: string, checkout: string) => Promise<void>) {
  const root = await mkdtemp(join(tmpdir(), "rerail-receipts-"));
  roots.push(root);
  const databasePath = join(root, "rerail.sqlite");
  const checkout = join(root, "checkout");
  await mkdir(checkout);
  const repositoryKey = startTestRepository(checkout, databasePath);
  await beforeStart?.(databasePath, checkout);
  const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-receipts-${process.pid}-${Date.now()}` : join(root, "engine.sock");
  const start = async (): Promise<EngineService> => {
    const service = await startEngineService({
      socketPath,
      storageRoot: join(root, "state"),
      databasePath,
      embedder: testEmbedder,
      ...(accessToken === undefined ? {} : { accessToken }),
      createAuditor: () => ({
        openWorkspace: async () => undefined,
        acceptsPath: () => true,
        auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] }),
        close: async () => undefined,
      }),
    });
    services.push(service);
    return service;
  };
  let service = await start();
  const startAgain = async (): Promise<void> => {
    services.splice(services.indexOf(service), 1);
    await service.close();
    service = await start();
  };
  const session = (sessionId: string) => ({ host: "claude-code" as const, sessionId, cwd: checkout });
  return { root, databasePath, repositoryKey, socketPath, session, startAgain };
}

/** Another connection, in this process, holding the write lock of the database at `databasePath` until `release`. */
function holdWriteLock(databasePath: string): () => void {
  const holder = new Database(databasePath);
  holder.exec("BEGIN IMMEDIATE");
  return () => {
    holder.exec("ROLLBACK");
    holder.close();
  };
}

/** Makes this process's record writes to `databasePath` still waiting for the lock: a read of what it recorded does. */
function recordsMade(databasePath: string): void {
  const store = new SqliteSessionStore(databasePath);
  try {
    store.sessions();
  } finally {
    store.close();
  }
}

/** The receipts the service keeps on disk beside the database at `databasePath` until their records are made. */
async function keptFiles(databasePath: string): Promise<string[]> {
  return await readdir(`${databasePath}-receipts`).catch(() => []);
}

/** When each session's row says it started. */
function startedAt(databasePath: string, sessionId: string): number | undefined {
  const database = new Database(databasePath, { readonly: true });
  try {
    return database.prepare<[string], { startedAt: number }>("SELECT started_at AS startedAt FROM sessions WHERE session_id = ?").get(sessionId)?.startedAt;
  } finally {
    database.close();
  }
}

/**
 * The steer receipts recorded in the database at `databasePath`. The service makes a receipt's record, or keeps it
 * until the write lock is given back, before it answers, so by the time the client here, in the same process, reads
 * the answer, a receipt made at once is on disk.
 */
function receipts(databasePath: string): Array<{ sessionId: string; eventId: string; payloadJson: string; repositoryId: string | null }> {
  const database = new Database(databasePath, { readonly: true });
  try {
    return database.prepare<[], { sessionId: string; eventId: string; payloadJson: string; repositoryId: string | null }>(
      "SELECT e.session_id AS sessionId, e.event_id AS eventId, e.payload_json AS payloadJson, s.repository_id AS repositoryId "
        + "FROM events e JOIN sessions s ON s.session_id = e.session_id WHERE e.kind = 'steering-delivered' ORDER BY e.sequence",
    ).all();
  } finally {
    database.close();
  }
}

describe("the engine service's receipts", () => {
  it("records a receipt a hook hands over in its session's runtime, once however often it is handed", async () => {
    const { databasePath, repositoryKey, socketPath, session } = await serviceOverCheckout();
    const link = engineLink({ socketPath });
    // The runtime this ingest builds reads the session's row on an interval timer: faked here, it cannot tick while
    // the row is gone, where it would throw from the timer and fail the run.
    vi.useFakeTimers({ toFake: ["Date", "setInterval", "clearInterval"], now: 1_000_000 });
    await link.ingest({ kind: "session-start", eventId: "a:start", session: session("a") });
    // A reset removed the session's row: the runtime that holds the session puts it back with its own start, where a
    // record on connections of the service's own would date the session from the receipt.
    const reset = new Database(databasePath);
    reset.pragma("foreign_keys = OFF");
    reset.prepare("DELETE FROM sessions WHERE session_id = ?").run("a");
    reset.close();
    vi.setSystemTime(5_000_000);
    const receipt = { session: session("a"), receiptId: "receipt-1", ruleIds: ["rule:a", "rule:b"] };
    await expect(link.handReceipt(receipt)).resolves.toBe(true);
    await expect(link.handReceipt(receipt)).resolves.toBe(true);
    expect(receipts(databasePath)).toEqual([
      { sessionId: "a", eventId: "steering-delivered:receipt-1", payloadJson: JSON.stringify({ ruleIds: ["rule:a", "rule:b"] }), repositoryId: repositoryKey },
    ]);
    expect(startedAt(databasePath, "a")).toBe(1_000_000);
  });

  it.each([["in its session's runtime", true], ["for a session no runtime holds", false]] as const)(
    "takes a receipt while another connection holds the write lock, its thread never waiting, keeps it on disk, and records it once the lock is given back (%s)",
    async (_case, started) => {
      const { databasePath, repositoryKey, socketPath, session } = await serviceOverCheckout();
      const link = engineLink({ socketPath });
      if (started) await link.ingest({ kind: "session-start", eventId: "b:start", session: session("b") });
      // Held on this thread, which also runs the service: a record that waited for the lock here could never get it.
      const release = holdWriteLock(databasePath);
      try {
        await expect(link.handReceipt({ session: session("b"), receiptId: "receipt-4", ruleIds: ["rule:a"] })).resolves.toBe(true);
        expect(receipts(databasePath)).toEqual([]);
        // The hook has ended: a daemon that died now would leave the receipt for the next one to record.
        expect(await keptFiles(databasePath)).toHaveLength(1);
      } finally {
        release();
      }
      recordsMade(databasePath);
      expect(receipts(databasePath)).toEqual([
        { sessionId: "b", eventId: "steering-delivered:receipt-4", payloadJson: JSON.stringify({ ruleIds: ["rule:a"] }), repositoryId: repositoryKey },
      ]);
      expect(await keptFiles(databasePath)).toEqual([]);
    },
  );

  it.each([["in its session's runtime", true], ["for a session no runtime holds", false]] as const)(
    "keeps a taken receipt whose record fails once the lock is given back on disk, and the next start records it (%s)",
    async (_case, started) => {
      const { databasePath, socketPath, session, startAgain } = await serviceOverCheckout();
      const link = engineLink({ socketPath });
      if (started) await link.ingest({ kind: "session-start", eventId: "d:start", session: session("d") });
      const holder = new Database(databasePath);
      holder.exec("BEGIN IMMEDIATE");
      await expect(link.handReceipt({ session: session("d"), receiptId: "receipt-8", ruleIds: ["rule:a"] })).resolves.toBe(true);
      // The holder leaves the database refusing the receipt's insert, as a full disk or a read-only file would.
      holder.exec("CREATE TRIGGER refuse_steers BEFORE INSERT ON events WHEN NEW.kind = 'steering-delivered' BEGIN SELECT RAISE(ABORT, 'database or disk is full'); END");
      holder.exec("COMMIT");
      recordsMade(databasePath);
      expect(receipts(databasePath)).toEqual([]);
      expect(await keptFiles(databasePath)).toHaveLength(1);
      holder.exec("DROP TRIGGER refuse_steers");
      holder.close();
      await startAgain();
      recordsMade(databasePath);
      expect(receipts(databasePath).map(row => row.eventId)).toEqual(["steering-delivered:receipt-8"]);
      expect(await keptFiles(databasePath)).toEqual([]);
    },
  );

  it("records at its start each receipt a daemon took and ended before recording, once, and removes what it kept", async () => {
    const receipt = (checkout: string) => ({ session: { host: "claude-code" as const, sessionId: "left", cwd: checkout }, receiptId: "receipt-6", ruleIds: ["rule:a"] });
    const { databasePath, repositoryKey } = await serviceOverCheckout(undefined, async (databasePath, checkout) => {
      // What a daemon killed while another process held the write lock leaves: the receipt it took, kept but never
      // recorded, and a file whose write its end cut short.
      await keepReceipt(databasePath, receipt(checkout));
      await keepReceipt(databasePath, receipt(checkout));
      await writeFile(join(`${databasePath}-receipts`, "cut-short.json"), "{\"session\":");
    });
    recordsMade(databasePath);
    expect(receipts(databasePath)).toEqual([
      { sessionId: "left", eventId: "steering-delivered:receipt-6", payloadJson: JSON.stringify({ ruleIds: ["rule:a"] }), repositoryId: repositoryKey },
    ]);
    expect(await keptFiles(databasePath)).toEqual([]);
  });

  it("leaves a kept receipt whose read fails at its start for the next start, which records it", async () => {
    const receipt = (checkout: string) => ({ session: { host: "claude-code" as const, sessionId: "left", cwd: checkout }, receiptId: "receipt-7", ruleIds: ["rule:a"] });
    let refused = 0;
    const { databasePath, repositoryKey, startAgain } = await serviceOverCheckout(undefined, async (databasePath, checkout) => {
      await keepReceipt(databasePath, receipt(checkout));
      // Another process holds the kept file at the start's read, once, as a scanner on Windows does.
      vi.mocked(readFileSync).mockImplementation((path, options) => {
        if (refused === 0 && String(path).startsWith(`${databasePath}-receipts`)) {
          refused += 1;
          throw Object.assign(new Error(`EBUSY: resource busy or locked, open '${String(path)}'`), { code: "EBUSY" });
        }
        return real.readFileSync(path, options);
      });
    });
    recordsMade(databasePath);
    expect(refused).toBe(1);
    expect(receipts(databasePath)).toEqual([]);
    expect(await keptFiles(databasePath)).toHaveLength(1);
    await startAgain();
    recordsMade(databasePath);
    expect(receipts(databasePath)).toEqual([
      { sessionId: "left", eventId: "steering-delivered:receipt-7", payloadJson: JSON.stringify({ ruleIds: ["rule:a"] }), repositoryId: repositoryKey },
    ]);
    expect(await keptFiles(databasePath)).toEqual([]);
  });

  it("answers a receipt it could not record with why, so the hook records it itself", async () => {
    const { root, databasePath, socketPath, session } = await serviceOverCheckout();
    const notADatabase = join(root, "not-a-database.sqlite");
    await writeFile(notADatabase, "not SQLite");
    const configuration = { storageRoot: join(root, "state"), modelPath: join(root, "model"), databasePath: notADatabase };
    await expect(engineLink({ socketPath }).handReceipt({ session: session("c"), receiptId: "receipt-5", ruleIds: [] }, configuration)).resolves.toBe(false);
    expect(receipts(databasePath)).toEqual([]);
  });

  it("records a receipt for a session no runtime holds in the database, crediting its checkout's repository", async () => {
    const { databasePath, repositoryKey, socketPath, session } = await serviceOverCheckout();
    // A session this daemon never opened: its runtime retired, or the daemon started after its event.
    await engineLink({ socketPath }).handReceipt({ session: session("gone"), receiptId: "receipt-2", ruleIds: [] });
    expect(receipts(databasePath)).toEqual([
      { sessionId: "gone", eventId: "steering-delivered:receipt-2", payloadJson: JSON.stringify({ ruleIds: [] }), repositoryId: repositoryKey },
    ]);
  });

  it("refuses a receipt without the daemon's capability and records nothing", async () => {
    const { databasePath, socketPath, session } = await serviceOverCheckout("capability");
    const receipt = { session: session("a"), receiptId: "receipt-3", ruleIds: ["rule:a"] };
    await expect(engineLink({ socketPath }).handReceipt(receipt)).resolves.toBe(false);
    expect(receipts(databasePath)).toEqual([]);
    await engineLink({ socketPath, accessToken: "capability" }).handReceipt(receipt);
    expect(receipts(databasePath)).toHaveLength(1);
  });
});
