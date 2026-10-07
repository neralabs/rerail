import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import { SqlitePolicyExecutionReader, SqlitePolicyExecutionStore } from "../src/policy-execution-store.js";

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
  mode: "resteer" as const,
  revision: 3,
  catalogRevision: null,
  ruleRevisions: [],
  paths: ["src/a.ts"],
  ruleProvenance: [{
    ruleId: "policy-a",
    revision: 7,
    payloadHash: "sha256:policy-a",
    applicability: "project" as const,
    projectId: "project-a",
    origin: "local" as const,
    syncState: "local-only" as const,
  }],
};

describe("SQLite policy execution persistence", () => {
  it("persists terminal findings and failure payloads across reopen", async () => {
    const path = await temporaryDatabase();
    const first = new SqlitePolicyExecutionStore(path);
    const original = first.start(input);
    first.finish(original, {
      status: "failed",
      findings: [
        {
          id: "finding-1",
          policyId: "policy-a",
          checkId: "check-a",
          code: "forbidden-call",
          severity: "error",
          message: "forbidden call",
          path: "src/a.ts",
          captures: {},
          variables: {},
        },
      ],
      failures: [{ checkId: "check-b", code: "adapter-error", message: "native evaluator exited", stage: "evaluate" }],
    });
    const followUp = first.start({ ...input, revision: 4 });
    first.finish(followUp, { status: "completed" });
    first.close();

    const reopened = new SqlitePolicyExecutionStore(path);
    expect(reopened.list()).toMatchObject({
      total: 2,
      runs: expect.arrayContaining([
        expect.objectContaining({
          runId: original,
          status: "failed",
          ruleProvenance: input.ruleProvenance,
          findings: [expect.objectContaining({ id: "finding-1", captures: {}, variables: {} })],
          failures: [
            { checkId: "check-b", code: "adapter-error", message: "native evaluator exited", stage: "evaluate" },
          ],
        }),
        expect.objectContaining({ runId: followUp, status: "completed", findings: [], failures: [] }),
      ]),
    });
    reopened.close();
  });

  it("interrupts only running work for the selected session and pages SQL-filtered history", async () => {
    const store = new SqlitePolicyExecutionStore(await temporaryDatabase());
    const interrupted = store.start(input);
    const otherSession = store.start({ ...input, sessionId: "session-b", scope: "repository", mode: "log", paths: ["src/b.ts"] });
    const alreadyFinished = store.start({ ...input, scope: "ai-output", mode: "log", paths: ["src/c.ts"] });
    store.finish(alreadyFinished, { status: "completed" });

    store.interrupt("session-a");
    store.finish(interrupted, { status: "completed" });

    expect(store.list({ sessionId: "session-a", status: "interrupted" })).toMatchObject({
      total: 1,

      runs: [expect.objectContaining({ runId: interrupted, status: "interrupted", findings: [], failures: [] })],
    });
    expect(store.list({ sessionId: "session-a", status: "completed" })).toMatchObject({
      total: 1,
      runs: [expect.objectContaining({ runId: alreadyFinished })],
    });
    expect(store.list({ sessionId: "session-b", status: "running", scope: "repository", mode: "log" })).toMatchObject({
      total: 1,
      runs: [expect.objectContaining({ runId: otherSession, scope: "repository", mode: "log" })],
    });
    expect(store.list({ sessionId: "session-b", scope: "repository", mode: "resteer" })).toEqual({ total: 0, runs: [] });
    expect(store.list({ sessionId: "session-a", limit: 1, offset: 1 })).toMatchObject({
      total: 2,
      runs: [expect.any(Object)],
    });
    store.close();
  });
  it("assembles bounded result pages in SQLite without a JavaScript result aggregate", async () => {
    const path = await temporaryDatabase();
    const store = new SqlitePolicyExecutionStore(path);
    const runId = store.start(input);
    const finding = {
      id: "finding-1",
      policyId: "policy-a",
      checkId: "check-a",
      code: "forbidden-call",
      severity: "error" as const,
      message: "forbidden call",
      path: "src/a.ts",
      captures: {},
      variables: {},
    };
    store.appendResultPage(runId, 0, { findings: [finding], failures: [] });
    store.appendResultPage(runId, 1, {
      findings: [{ ...finding, id: "finding-2", path: "src/b.ts" }],
      failures: [{ checkId: "check-b", code: "adapter-error", message: "failed", stage: "evaluate" }],
    });
    store.finishResultPages(runId, "failed");

    expect(store.get(runId)).toMatchObject({
      status: "failed",
      findings: [{ id: "finding-1" }, { id: "finding-2" }],
      failures: [{ checkId: "check-b" }],
    });
    expect(new Database(path).prepare("SELECT count(*) AS count FROM policy_run_result_pages").get()).toEqual({ count: 0 });
    store.close();
  });

  it("keeps a read-only listing empty until the database exists", async () => {
    const path = await temporaryDatabase();
    const reader = new SqlitePolicyExecutionReader(path);
    expect(reader.list()).toEqual({ runs: [], total: 0 });
    reader.close();
  });
});
