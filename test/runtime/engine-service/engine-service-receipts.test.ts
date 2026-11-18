import { mkdir, mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import { engineLink, startEngineService, type EngineService } from "../../../src/runtime/engine-service.js";
import { testEmbedder } from "../../support/hook-events.js";
import { startTestRepository } from "../../support/started-repository.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const roots: string[] = [];
const services: EngineService[] = [];
afterEach(async () => {
  await Promise.all(services.splice(0).map(service => service.close()));
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

/** An engine service over one started checkout, with the capability `accessToken` when one is given. */
async function serviceOverCheckout(accessToken?: string) {
  const root = await mkdtemp(join(tmpdir(), "rerail-receipts-"));
  roots.push(root);
  const databasePath = join(root, "rerail.sqlite");
  const checkout = join(root, "checkout");
  await mkdir(checkout);
  const repositoryKey = startTestRepository(checkout, databasePath);
  const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-receipts-${process.pid}-${Date.now()}` : join(root, "engine.sock");
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
  const session = (sessionId: string) => ({ host: "claude-code" as const, sessionId, cwd: checkout });
  return { databasePath, repositoryKey, socketPath, session };
}

/**
 * The steer receipts recorded in the database at `databasePath`. The service answers a receipt and then records it in
 * the same turn of its event loop, so by the time the client here, in the same process, reads the answer, the record is
 * made. A record that outlasts the client's bound on a slow machine (a disk that stalls, a session no runtime holds,
 * recorded on connections of its own) makes the client call it untaken, so these cases check the record, not the
 * client's answer: the record is what counts, and the hook's answer to a daemon that takes a receipt is checked against
 * an engine socket of the test's own (`test/harness/hook-steer-receipt.test.ts`).
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
    await link.ingest({ kind: "session-start", eventId: "a:start", session: session("a") });
    const receipt = { session: session("a"), receiptId: "receipt-1", ruleIds: ["rule:a", "rule:b"] };
    await link.handReceipt(receipt);
    await link.handReceipt(receipt);
    expect(receipts(databasePath)).toEqual([
      { sessionId: "a", eventId: "steering-delivered:receipt-1", payloadJson: JSON.stringify({ ruleIds: ["rule:a", "rule:b"] }), repositoryId: repositoryKey },
    ]);
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
