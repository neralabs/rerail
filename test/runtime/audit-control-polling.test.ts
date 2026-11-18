import { realpathSync } from "node:fs";
import { mkdir, realpath, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { eq } from "drizzle-orm";
import { afterEach, describe, expect, it, vi } from "vitest";
import { auditSettings, audits, codeRuleSchema, NativeCodeRuleEnforcer, openRerailDatabase, recordCheckout, SqliteAuditControlStore } from "../../src/engine/index.js";
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

describe.runIf(process.platform === "win32")("A session in a Windows folder outside Git spelt otherwise than the disk stores it", () => {
  /**
   * Log mode saved by Rerail 0.1.4 for a folder `App` reached as VS Code or a typed `cd` can spell it (`c:\…\app`):
   * under the JS realpath, which keeps that spelling, beside the checkout row 0.1.4 recorded under the same key when its
   * session opened there, naming the folder's own clone and repository, else `repositoryKey`'s. Then a session there writes a
   * file that breaks a rule, through a runtime of this build, and the mode each audit of that rule was checked in.
   */
  async function breakRuleWhere014SavedLog(repositoryKey?: string): Promise<{ deliveries: readonly unknown[]; modes: string[] }> {
    const root = await realpath(await temporaryDirectory());
    const stored = join(root, "App");
    await mkdir(stored);
    const spelled = join(dirname(stored).replace(/^[A-Z]:/u, drive => drive.toLowerCase()), "app");
    const earlierKey = realpathSync(spelled);
    expect(earlierKey).not.toBe(stored);
    const databasePath = join(root, "rerail.sqlite");
    const database = openRerailDatabase(databasePath);
    try {
      const recorded = recordCheckout(database, spelled);
      database.insert(auditSettings).values({ workspaceRoot: earlierKey, settings: { scope: "ai-output", mode: "log" }, updatedAt: 1 }).run();
      if (repositoryKey !== undefined) database.$client.prepare("INSERT INTO repositories (repository_key, display_name, started, configured_at, created_at, updated_at) VALUES (?, 'Another', 1, 1, 1, 1)").run(repositoryKey);
      database.$client.prepare("UPDATE checkouts SET checkout_path = ?, repository_key = ?, clone_id = ? WHERE checkout_path = ?")
        .run(earlierKey, repositoryKey ?? recorded.repositoryKey, repositoryKey ?? recorded.cloneId, stored);
    } finally { database.$client.close(); }
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"), databasePath, workspaceRoot: stored, autostart: false, embedder: testEmbedder,
      createAuditor: options => new NativeCodeRuleEnforcer(catalog(), options),
    });
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "omp", sessionId: "spelled", cwd: spelled } });
      await runtime.codeRulesOpened("spelled");
      await runtime.ingest({ kind: "call", eventId: "call", sessionId: "spelled", toolCallId: "w1", toolName: "write", input: { path: "created.py", content: "" } });
      await writeFile(join(stored, "created.py"), "NEEDLE = 1\n");
      const deliveries = await runtime.ingest({ kind: "result", eventId: "result", sessionId: "spelled", toolCallId: "w1", toolName: "write", output: "written" });
      const reader = openRerailDatabase(databasePath);
      try {
        const modes = reader.select({ mode: audits.mode, violations: audits.violations }).from(audits).where(eq(audits.sessionId, "spelled")).all()
          .filter(audit => audit.violations.some(violation => violation.ruleId === "test:needle")).map(audit => audit.mode);
        return { deliveries, modes };
      } finally { reader.$client.close(); }
    } finally {
      await runtime.dispose();
    }
  }

  it("checks in the Log mode Rerail 0.1.4 saved for that spelling, recording the broken rule without telling the coding agent", async () => {
    expect(await breakRuleWhere014SavedLog()).toEqual({ deliveries: [], modes: ["log"] });
  }, 20_000);

  it("checks in Steer where 0.1.4 saved Log mode under that spelling for another repository's folder, as a drive letter now leading elsewhere leaves it", async () => {
    expect((await breakRuleWhere014SavedLog("clone:another")).modes).toEqual(["steer"]);
  }, 20_000);
});
