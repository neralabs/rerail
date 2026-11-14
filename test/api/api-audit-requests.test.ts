import { realpathSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { setTimeout as delay } from "node:timers/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createRerailApi } from "../../src/api/http.js";
import { operations } from "../../src/api/operations.js";
import { DatabaseApiStores } from "../../src/api/stores.js";
import { removeFolder } from "../support/platform/remove-folder.js";

describe("a scan asked through the API", () => {
  let root: string;
  let stores: DatabaseApiStores;
  let resumed: string[];
  /** What opening a session again waits on: nothing, unless a test holds it. */
  let reopening: Promise<void>;

  beforeEach(async () => {
    root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-audit-requests-")));
    const databasePath = join(root, "rerail.sqlite");
    resumed = [];
    reopening = Promise.resolve();
    stores = new DatabaseApiStores(databasePath, {
      health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
      resumeSession: async (sessionId) => {
        resumed.push(sessionId);
        await reopening;
      },
    });
    for (const sessionId of ["owned", "ended"]) stores.sessionStore().openSession({ host: "claude-code", sessionId, cwd: root });
    stores.auditControls().registerOwner("owned", "next-event");
    stores.sessionStore().closeSession("ended");
  });

  afterEach(async () => {
    stores.close();
    await removeFolder(root);
  });

  async function requestScan(sessionId: string): Promise<{ status: string; error: string | null }> {
    const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    const response = await app.request(`/api/sessions/${sessionId}/audit-requests`, {
      method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ scope: "repository" }),
    });
    expect(response.status).toBe(200);
    // Test boundary: the operation's output schema is asserted field by field below.
    return await response.json() as { status: string; error: string | null };
  }

  // A session the daemon suspended while it was quiet keeps its owner, so its scan queues; the daemon opens it again
  // to run the scan now, as an open session would have.
  it("opens again the session a queued scan is for", async () => {
    expect(await requestScan("owned")).toMatchObject({ status: "queued", error: null });
    expect(resumed).toEqual(["owned"]);
  });

  // Opening a session again reads its ledger back and can make its checkout's runtime again; on main the request was
  // answered as soon as it was queued.
  it("answers once the scan is queued, while the session is still opening again", async () => {
    const opening = Promise.withResolvers<void>();
    reopening = opening.promise;
    try {
      const answer = await Promise.race([requestScan("owned"), delay(2_000, "no answer within 2 s")]);
      expect(answer).toMatchObject({ status: "queued", error: null });
      expect(resumed).toEqual(["owned"]);
    } finally {
      opening.resolve();
    }
  });

  it("opens no session for a scan it could not queue", async () => {
    expect(await requestScan("ended")).toMatchObject({ status: "failed", error: "Session is inactive" });
    expect(resumed).toEqual([]);
  });
});
