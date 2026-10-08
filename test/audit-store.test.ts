import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import { SqliteAuditReader, SqliteAuditStore } from "../src/audit-store.js";

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
});
