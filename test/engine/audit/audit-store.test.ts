import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import { SqliteAuditReader, SqliteAuditStore } from "../../../src/engine/audit/audit-store.js";

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map((path) => rm(path, { recursive: true, force: true })));
});

async function temporaryDatabase(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-policy-runs-"));
  temporaryDirectories.push(root);
  return join(root, ".rerail", "rerail.sqlite");
}

const input = {
  sessionId: "session-a",
  workspaceRoot: "/workspace",
  scope: "touched-files" as const,
  mode: "steer" as const,
  revision: 3,
  catalogRevision: null,
  ruleRevisions: [],
  paths: ["src/a.ts"],
  ruleProvenance: [{
    ruleId: "code-rule-a",
    revision: 7,
    payloadHash: "sha256:code-rule-a",
    applicability: "project" as const,
    projectId: "project-a",
    origin: "user" as const,
  }],
};

describe("SQLite audit persistence", () => {
  it("persists terminal violations and failure payloads across reopen", async () => {
    const path = await temporaryDatabase();
    const first = new SqliteAuditStore(path);
    const original = first.start(input);
    first.finish(original, {
      status: "failed",
      violations: [
        {
          id: "violation-1",
          ruleId: "code-rule-a",
          clauseId: "clause-a",
          code: "forbidden-call",
          mode: "block",
          message: "forbidden call",
          path: "src/a.ts",
          captures: {},
          variables: {},
        },
      ],
      failures: [{ clauseId: "clause-b", code: "adapter-error", message: "native evaluator exited", stage: "evaluate" }],
    });
    const followUp = first.start({ ...input, revision: 4 });
    first.finish(followUp, { status: "completed" });
    first.close();

    const reopened = new SqliteAuditStore(path);
    expect(reopened.list()).toMatchObject({
      total: 2,
      audits: expect.arrayContaining([
        expect.objectContaining({
          auditId: original,
          status: "failed",
          ruleProvenance: input.ruleProvenance,
          violations: [expect.objectContaining({ id: "violation-1", captures: {}, variables: {} })],
          failures: [
            { clauseId: "clause-b", code: "adapter-error", message: "native evaluator exited", stage: "evaluate" },
          ],
        }),
        expect.objectContaining({ auditId: followUp, status: "completed", violations: [], failures: [] }),
      ]),
    });
    reopened.close();
  });

  it("interrupts only running work for the selected session and pages SQL-filtered history", async () => {
    const store = new SqliteAuditStore(await temporaryDatabase());
    const interrupted = store.start(input);
    const otherSession = store.start({ ...input, sessionId: "session-b", scope: "repository", mode: "log", paths: ["src/b.ts"] });
    const alreadyFinished = store.start({ ...input, scope: "ai-output", mode: "log", paths: ["src/c.ts"] });
    store.finish(alreadyFinished, { status: "completed" });

    store.interrupt("session-a");
    store.finish(interrupted, { status: "completed" });

    expect(store.list({ sessionId: "session-a", status: "interrupted" })).toMatchObject({
      total: 1,

      audits: [expect.objectContaining({ auditId: interrupted, status: "interrupted", violations: [], failures: [] })],
    });
    expect(store.list({ sessionId: "session-a", status: "completed" })).toMatchObject({
      total: 1,
      audits: [expect.objectContaining({ auditId: alreadyFinished })],
    });
    expect(store.list({ sessionId: "session-b", status: "running", scope: "repository", mode: "log" })).toMatchObject({
      total: 1,
      audits: [expect.objectContaining({ auditId: otherSession, scope: "repository", mode: "log" })],
    });
    expect(store.list({ sessionId: "session-b", scope: "repository", mode: "steer" })).toEqual({ total: 0, audits: [] });
    expect(store.list({ sessionId: "session-a", limit: 1, offset: 1 })).toMatchObject({
      total: 2,
      audits: [expect.any(Object)],
    });
    store.close();
  });
  it("assembles bounded result pages in SQLite without a JavaScript result aggregate", async () => {
    const path = await temporaryDatabase();
    const store = new SqliteAuditStore(path);
    const auditId = store.start(input);
    const violation = {
      id: "violation-1",
      ruleId: "code-rule-a",
      clauseId: "clause-a",
      code: "forbidden-call",
      mode: "steer" as const,
      message: "forbidden call",
      path: "src/a.ts",
      captures: {},
      variables: {},
    };
    store.appendResultPage(auditId, 0, { violations: [violation], failures: [] });
    store.appendResultPage(auditId, 1, {
      violations: [{ ...violation, id: "violation-2", path: "src/b.ts" }],
      failures: [{ clauseId: "clause-b", code: "adapter-error", message: "failed", stage: "evaluate" }],
    });
    store.finishResultPages(auditId, "failed");

    expect(store.get(auditId)).toMatchObject({
      status: "failed",
      violations: [{ id: "violation-1" }, { id: "violation-2" }],
      failures: [{ clauseId: "clause-b" }],
    });
    expect(new Database(path).prepare("SELECT count(*) AS count FROM audit_result_pages").get()).toEqual({ count: 0 });
    store.close();
  });



  it("keeps a read-only listing empty until the database exists", async () => {
    const path = await temporaryDatabase();
    const reader = new SqliteAuditReader(path);
    expect(reader.list()).toEqual({ audits: [], total: 0 });
    reader.close();
  });

  it("reads, read-only, a database only an earlier build wrote, which has no unlisted-failure table", async () => {
    const path = await temporaryDatabase();
    const store = new SqliteAuditStore(path);
    const auditId = store.start(input);
    store.finish(auditId, { status: "failed", failures: [{ clauseId: "c", code: "adapter-error", message: "m" }] });
    store.close();
    recordRepositoryAudit(path, input.workspaceRoot, auditId);
    const earlier = new Database(path);
    earlier.exec("DROP TABLE audit_unlisted_failures");
    earlier.close();
    const reader = new SqliteAuditReader(path);
    const counts = { failureCount: 1, unlistedFailureCount: 0 };
    expect(reader.get(auditId)).toMatchObject(counts);
    expect(reader.list().audits).toEqual([expect.objectContaining(counts)]);
    expect(reader.latestSummary("session-a")).toMatchObject(counts);
    expect(reader.listRepositoryAudits()).toMatchObject({ total: 1, repositories: [{ audit: counts }] });
    reader.close();
  });

  it("keeps a running audit's unlisted count with its progress, and replaces it when the audit finishes", async () => {
    const path = await temporaryDatabase();
    const store = new SqliteAuditStore(path);
    const failures = [{ clauseId: "c", code: "adapter-error" as const, message: "m" }];
    const auditId = store.start(input);
    store.progress(auditId, { violations: [], failures, unlistedFailureCount: 30 });
    expect(store.get(auditId)).toMatchObject({ status: "running", failureCount: 31, unlistedFailureCount: 30 });
    store.progress(auditId, { violations: [], failures, unlistedFailureCount: 45 });
    expect(store.get(auditId)).toMatchObject({ failureCount: 46, unlistedFailureCount: 45 });
    store.finish(auditId, { status: "failed", failures });
    expect(store.get(auditId)).toMatchObject({ status: "failed", failureCount: 1, unlistedFailureCount: 0 });
    const interrupted = store.start(input);
    store.progress(interrupted, { violations: [], failures, unlistedFailureCount: 9 });
    store.interrupt("session-a");
    expect(store.get(interrupted)).toMatchObject({ status: "interrupted", failureCount: 10, unlistedFailureCount: 9 });
    store.close();
  });

  it("counts each audit's own unlisted failures, never another audit's", async () => {
    const path = await temporaryDatabase();
    const store = new SqliteAuditStore(path);
    const failures = [{ clauseId: "c", code: "adapter-error" as const, message: "m" }];
    const many = store.start({ ...input, sessionId: "many", workspaceRoot: "/many" });
    store.finish(many, { status: "failed", failures, unlistedFailureCount: 30 });
    const listed = store.start({ ...input, sessionId: "listed", workspaceRoot: "/listed" });
    store.finish(listed, { status: "failed", failures: [...failures, ...failures, ...failures] });
    recordRepositoryAudit(path, "/many", many);
    recordRepositoryAudit(path, "/listed", listed);
    for (const reader of [store, new SqliteAuditReader(path)]) {
      expect(reader.get(many)).toMatchObject({ failureCount: 31, unlistedFailureCount: 30 });
      expect(reader.get(listed)).toMatchObject({ failureCount: 3, unlistedFailureCount: 0 });
      expect(reader.list().audits.map(({ auditId, failureCount }) => [auditId, failureCount]).sort())
        .toEqual([[listed, 3], [many, 31]].sort());
      expect(reader.latestSummary("many")).toMatchObject({ failureCount: 31, unlistedFailureCount: 30 });
      expect(reader.latestSummary("listed")).toMatchObject({ failureCount: 3, unlistedFailureCount: 0 });
      expect(reader.listRepositoryAudits().repositories.map(({ workspaceRoot, audit }) =>
        [workspaceRoot, audit?.failureCount, audit?.unlistedFailureCount])).toEqual([["/listed", 3, 0], ["/many", 31, 30]]);
    }
    store.close();
  });
});

/** Records `auditId` as `workspaceRoot`'s repository audit, as the audit control does. */
function recordRepositoryAudit(path: string, workspaceRoot: string, auditId: string): void {
  const database = new Database(path);
  database.prepare(`INSERT INTO repository_audits (workspace_root, session_id, audit_id, status, started_at)
    VALUES (?, 'session-a', ?, 'failed', 1)`).run(workspaceRoot, auditId);
  database.close();
}
