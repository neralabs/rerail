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
    origin: "migrated" as const,
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

  it("upgrades a stamped v3 database additively without disturbing its session rows", async () => {
    const path = await temporaryDatabase();
    const bootstrap = new SqlitePolicyExecutionStore(path);
    bootstrap.close();
    const raw = new Database(path);
    raw.exec(
      "INSERT INTO sessions (session_id, host, cwd, started_at, ended_at) VALUES ('live', 'omp', '/workspace', 1, NULL); DROP TABLE policy_runs; PRAGMA user_version = 3",
    );
    raw.close();

    const upgraded = new SqlitePolicyExecutionStore(path);
    expect(upgraded.start({ ...input, sessionId: "live" })).toEqual(expect.any(String));
    upgraded.close();

    const verified = new Database(path, { readonly: true });
    expect(verified.prepare("SELECT session_id FROM sessions").all()).toEqual([{ session_id: "live" }]);
    expect(verified.prepare("SELECT count(*) AS total FROM policy_runs").get()).toEqual({ total: 1 });
    verified.close();
  });
  it("migrates v4 execution history to scope and mode without retaining delivery rows", async () => {
    const path = await temporaryDatabase();
    const bootstrap = new SqlitePolicyExecutionStore(path);
    bootstrap.close();
    const raw = new Database(path);
    raw.exec(
      "DROP TABLE policy_runs; " +
        "CREATE TABLE policy_runs (" +
        "run_id text PRIMARY KEY NOT NULL, session_id text NOT NULL, workspace_root text NOT NULL, kind text NOT NULL, " +
        "revision integer NOT NULL, scope_json text NOT NULL, retry_of text, started_at integer NOT NULL, finished_at integer, " +
        "status text NOT NULL, findings_json text NOT NULL, failures_json text NOT NULL" +
        "); " +
        "INSERT INTO policy_runs VALUES " +
        "('evaluation', 'live', '/workspace', 'scoped-scan', 3, '[\"src/a.ts\"]', NULL, 1, 2, 'completed', '[]', '[]'), " +
        "('handoff', 'live', '/workspace', 'delivery', 3, '[]', 'evaluation', 3, 4, 'failed', '[]', '[]'); " +
        "PRAGMA user_version = 4",
    );
    raw.close();

    const upgraded = new SqlitePolicyExecutionStore(path);
    expect(upgraded.list()).toEqual({
      total: 1,
      runs: [expect.objectContaining({
        runId: "evaluation",
        scope: "touched-files",
        mode: "log",
        paths: ["src/a.ts"],
        ruleProvenance: [],
        status: "completed",
        findings: [],
        failures: [],
      })],
    });
    upgraded.close();
  });

  it("migrates v5 kind history to independent scope and mode axes", async () => {
    const path = await temporaryDatabase();
    const bootstrap = new SqlitePolicyExecutionStore(path);
    bootstrap.close();
    const raw = new Database(path);
    raw.exec(
      "DROP TABLE policy_runs; " +
        "CREATE TABLE policy_runs (" +
        "run_id text PRIMARY KEY NOT NULL, session_id text NOT NULL, workspace_root text NOT NULL, kind text NOT NULL, " +
        "revision integer NOT NULL, scope_json text NOT NULL, started_at integer NOT NULL, finished_at integer, " +
        "status text NOT NULL, findings_json text NOT NULL, failures_json text NOT NULL" +
        "); " +
        "INSERT INTO policy_runs VALUES " +
        "('generation', 'live', '/workspace', 'generation', 3, '[\"src/a.ts\"]', 1, 2, 'completed', '[{\"id\":\"f\"}]', '[]'), " +
        "('ratchet', 'live', '/workspace', 'ratchet', 3, '[\"src/b.ts\"]', 3, 4, 'failed', '[]', '[{\"checkId\":\"scope\",\"code\":\"source-incomplete\",\"message\":\"missing\"}]'); " +
        "PRAGMA user_version = 5",
    );
    raw.close();

    const upgraded = new SqlitePolicyExecutionStore(path);
    expect(upgraded.list({ scope: "touched-files", mode: "resteer" }).runs).toMatchObject([{
      runId: "generation",
      paths: ["src/a.ts"],
      ruleProvenance: [],
      findings: [{ id: "f" }],
    }]);
    expect(upgraded.list({ scope: "ai-output", mode: "log" }).runs).toMatchObject([{
      runId: "ratchet",
      paths: ["src/b.ts"],
      ruleProvenance: [],
      failures: [{ checkId: "scope", code: "source-incomplete", message: "missing" }],
    }]);
    upgraded.close();
  });

  it("upgrades v6 history to v7 provenance columns without losing completed runs", async () => {
    const path = await temporaryDatabase();
    const bootstrap = new SqlitePolicyExecutionStore(path);
    bootstrap.close();
    const raw = new Database(path);
    raw.exec(
      "DROP TABLE policy_runs; " +
        "CREATE TABLE policy_runs (" +
        "run_id text PRIMARY KEY NOT NULL, session_id text NOT NULL, workspace_root text NOT NULL, scope text NOT NULL, mode text NOT NULL, " +
        "revision integer NOT NULL, paths_json text NOT NULL, started_at integer NOT NULL, finished_at integer, status text NOT NULL, findings_json text NOT NULL, failures_json text NOT NULL" +
        "); INSERT INTO policy_runs VALUES ('historic', 'live', '/workspace', 'repository', 'log', 2, '[]', 1, 2, 'completed', '[]', '[]'); PRAGMA user_version = 6",
    );
    raw.close();

    const upgraded = new SqlitePolicyExecutionStore(path);
    expect(upgraded.get("historic")).toMatchObject({ runId: "historic", status: "completed", catalogRevision: null, ruleRevisions: [] });
    const runId = upgraded.start({ ...input, catalogRevision: "catalog-v7", ruleRevisions: [{ ruleId: "rule-a", revision: 4 }] });
    expect(upgraded.get(runId)).toMatchObject({ catalogRevision: "catalog-v7", ruleRevisions: [{ ruleId: "rule-a", revision: 4 }] });
    upgraded.close();
  });

  it("keeps a read-only listing empty until the database exists", async () => {
    const path = await temporaryDatabase();
    const reader = new SqlitePolicyExecutionReader(path);
    expect(reader.list()).toEqual({ runs: [], total: 0 });
    reader.close();
  });
});
