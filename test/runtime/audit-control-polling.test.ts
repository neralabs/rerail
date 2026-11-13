import { realpath } from "node:fs/promises";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { afterEach, describe, expect, it, vi } from "vitest";
import { codeRuleSchema, NativeCodeRuleEnforcer, SqliteAuditControlStore } from "../../src/engine/index.js";
import { RerailRuntime } from "../../src/runtime/runtime.js";
import { temporaryDirectories, testEmbedder } from "../support/hook-events.js";
import { testCodeRuleCatalog } from "../support/repository-rules.js";

const temporaryDirectory = temporaryDirectories("rerail-control-polling-");

afterEach(() => {
  vi.restoreAllMocks();
});

function catalog() {
  return testCodeRuleCatalog([codeRuleSchema.parse({
    format: "rerail.policy/v1", id: "test:needle", revision: 1, mode: "steer",
    scope: { include: ["**/*.py"], exclude: [] },
    clauses: [{ id: "needle", violation: { code: "NEEDLE", anchor: "witness.span", message: "No needle" }, forbid: { regex: { pattern: "NEEDLE", source: "file.content" } } }],
  })]);
}

async function idleRuntime(sessions: readonly string[]) {
  const root = await realpath(await temporaryDirectory());
  const databasePath = join(root, "rerail.sqlite");
  const runtime = new RerailRuntime({
    storageRoot: join(root, "sessions"), databasePath, workspaceRoot: root, autostart: false, embedder: testEmbedder,
    createAuditor: options => new NativeCodeRuleEnforcer(catalog(), options),
  });
  for (const sessionId of sessions) {
    await runtime.ingest({ kind: "session-start", eventId: `start-${sessionId}`, session: { host: "claude-code", sessionId, cwd: root } });
    await runtime.codeRulesOpened(sessionId);
  }
  return { runtime, databasePath };
}

describe("The runtime's audit control polling", () => {
  // The daemon holds every open session of the machine; reading each one's settings and queue four times a second
  // kept an idle daemon at about 1.5% of a core with three sessions open, and more with each session.
  it("reads no idle session's settings or audit requests until another connection commits", async () => {
    const { runtime, databasePath } = await idleRuntime(["a", "b", "c"]);
    try {
      // Every session is read once after it opens.
      await sleep(600);
      const claims = vi.spyOn(SqliteAuditControlStore.prototype, "claim");
      const settings = vi.spyOn(SqliteAuditControlStore.prototype, "hasUserSettings");
      await sleep(1_500);
      expect(claims.mock.calls.length + settings.mock.calls.length).toBe(0);

      // `rerail scan --session` queues through its own connection; the next tick claims it.
      const other = new SqliteAuditControlStore(databasePath);
      try {
        const queued = other.requestAudit("b", { scope: "repository" });
        await vi.waitFor(() => {
          expect(other.listAuditRequests("b").find(request => request.id === queued.id)?.status).not.toBe("queued");
        }, { timeout: 3_000, interval: 50 });
        expect(claims.mock.calls.map(([sessionId]) => sessionId)).toContain("b");
      } finally {
        other.close();
      }
    } finally {
      await runtime.dispose();
    }
  }, 20_000);

  it("claims a second request queued behind a running one once the first finishes, with no other commit", async () => {
    const { runtime, databasePath } = await idleRuntime(["a"]);
    const other = new SqliteAuditControlStore(databasePath);
    try {
      await sleep(600);
      // Both are visible to one tick; that tick claims only the oldest.
      const first = other.requestAudit("a", { scope: "repository" });
      const second = other.requestAudit("a", { scope: "repository" });
      await vi.waitFor(() => {
        const statuses = other.listAuditRequests("a").filter(request => [first.id, second.id].includes(request.id)).map(request => request.status);
        expect(statuses.every(status => status !== "queued" && status !== "running")).toBe(true);
      }, { timeout: 10_000, interval: 50 });
    } finally {
      other.close();
      await runtime.dispose();
    }
  }, 20_000);
});
