import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from "vitest";
import { createRerailApi } from "../src/api/http.js";
import { operations, type RerailApiStores } from "../src/api/operations.js";
import { NativeExecutablePolicyEnforcer } from "../src/executable-policy-enforcer.js";
import type { PolicyExecutionRecorder, PolicyRunInput, PolicyRunResult } from "../src/policy-execution.js";
import { bootstrapPolicyDslCatalog } from "../src/policy-dsl-catalog.js";
import { policyRuleSchema, type PolicyRule } from "../src/policy-schema.js";
import { SqliteRuleStore } from "../src/rule-store.js";
import { ScopedRuleCatalog } from "../src/scoped-rule-catalog.js";

function policy(id: string): PolicyRule {
  return policyRuleSchema.parse({
    format: "rerail.policy/v1", id, revision: 1,
    scope: { include: ["api/**/*.py"], exclude: [] },
    checks: [{ id: "needle", finding: { code: "NEEDLE", severity: "error", anchor: "witness.span", message: "No NEEDLE" }, forbid: { regex: { pattern: "NEEDLE", source: "file.content" } } }],
  });
}

function recordingRuns(): { recorder: PolicyExecutionRecorder; runs: Map<string, PolicyRunInput & { result?: PolicyRunResult }> } {
  const runs = new Map<string, PolicyRunInput & { result?: PolicyRunResult }>();
  let sequence = 0;
  return {
    recorder: {
      start(input) {
        const runId = `run-${++sequence}`;
        runs.set(runId, { ...input });
        return runId;
      },
      finish(runId, result) { runs.get(runId)!.result = result; },
      progress(runId, result) { runs.get(runId)!.result = { status: "completed", ...result }; },
      interrupt() {},
    },
    runs,
  };
}

let root: string;
let warnings: MockInstance<typeof process.stderr.write>;

beforeEach(async () => {
  root = await mkdtemp(join(tmpdir(), "rerail-quarantine-"));
  warnings = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
});

afterEach(async () => {
  warnings.mockRestore();
  await rm(root, { recursive: true, force: true });
});

/** A per-project store holding `test:kept` and `test:broken`, whose row no longer validates. */
function storeWithBrokenRow(): string {
  const databasePath = join(root, ".rerail", "policies.sqlite");
  const store = new SqliteRuleStore(databasePath);
  store.upsertPolicy(policy("test:kept"));
  store.upsertPolicy(policy("test:broken"));
  store.close();
  const raw = new Database(databasePath);
  raw.prepare("UPDATE rules SET checks_json = ? WHERE id = ?").run(JSON.stringify([{ id: "x" }]), "test:broken");
  raw.close();
  return databasePath;
}

const quarantineFailure = {
  policyId: "test:broken",
  checkId: "catalog",
  code: "adapter-error",
  stage: "parse",
  message: expect.stringMatching(/^policy rule test:broken \(repository\) is skipped: checks\/0/u),
};

describe("quarantined policy rows", () => {
  it("are listed by the compiled catalog for the load that skipped them", () => {
    const catalog = bootstrapPolicyDslCatalog({ databasePath: storeWithBrokenRow() });
    try {
      expect(catalog.policies().map(entry => entry.id)).toEqual(["test:kept"]);
      expect(catalog.quarantined()).toEqual([
        { ruleId: "test:broken", kind: "policy", owner: "repository", reason: expect.stringMatching(/checks/u) },
      ]);
    } finally {
      catalog.close();
    }
  });

  it("surface as one adapter-error per rule per catalog load, and clear once the row is fixed", async () => {
    const databasePath = storeWithBrokenRow();
    await mkdir(join(root, "api"));
    const catalog = bootstrapPolicyDslCatalog({ databasePath });
    const { recorder, runs } = recordingRuns();
    const enforcer = new NativeExecutablePolicyEnforcer(catalog, {
      workspaceRoot: root,
      delivery: { requestDelivery: async () => undefined },
      executions: recorder,
      mode: "log",
    });
    const reports = () => [...runs.values()].filter(run => run.result?.failures?.some(failure => failure.policyId === "test:broken"));
    try {
      await enforcer.openWorkspace("session");
      expect(enforcer.snapshotStatus().failures).toEqual([quarantineFailure]);
      expect(reports()).toEqual([expect.objectContaining({ paths: [], result: { status: "failed", findings: [], failures: [quarantineFailure] } })]);

      for (const revision of [1, 2]) {
        await enforcer.evaluateGeneratedEdits("session", {
          revision, checkpoints: ["edit"], edits: [{ path: "api/a.py", language: "python", generatedText: `NEEDLE = ${revision}\n` }],
        });
        await enforcer.settle();
      }
      expect(reports()).toHaveLength(1);
      expect(enforcer.snapshotStatus().failures).toEqual([quarantineFailure]);

      const store = new SqliteRuleStore(databasePath);
      store.upsertPolicy(policy("test:broken"));
      store.close();
      await enforcer.evaluateGeneratedEdits("session", {
        revision: 3, checkpoints: ["edit"], edits: [{ path: "api/a.py", language: "python", generatedText: "NEEDLE = 3\n" }],
      });
      await enforcer.settle();
      expect(enforcer.snapshotStatus().failures).toEqual([]);
      expect(reports()).toHaveLength(1);
    } finally {
      await enforcer.close();
      catalog.close();
    }
  });

  it("are listed beside the rules by rules_list", async () => {
    const catalog = ScopedRuleCatalog.open(join(root, "catalog.sqlite"), { repositoryId: "repo", workspaceRoot: root });
    try {
      catalog.upsertPolicy(policy("test:kept"));
      catalog.upsertPolicy(policy("test:broken"));
      const raw = new Database(join(root, "catalog.sqlite"));
      raw.prepare("UPDATE catalog_rules SET document_json = json_set(document_json, '$.rule.checks', json(?)) WHERE rule_id = ?")
        .run(JSON.stringify([{ id: "x" }]), "test:broken");
      raw.close();
      const app = createRerailApi({ rules: () => catalog } as unknown as RerailApiStores, operations, { version: "0.0.0-test" });

      const listed = await (await app.request("/api/rules")).json() as { rules: { rule: { id: string } }[]; quarantined: unknown[] };
      expect(listed.rules.map(stored => stored.rule.id)).toEqual(["test:kept"]);
      expect(listed.quarantined).toEqual([{
        ruleId: "test:broken",
        kind: "policy",
        owner: `project:${catalog.context().projectId}`,
        reason: expect.stringMatching(/checks/u),
      }]);
      const cosineOnly = await (await app.request("/api/rules?kind=cosine")).json() as { quarantined: unknown[] };
      expect(cosineOnly.quarantined).toEqual([]);
    } finally {
      catalog.close();
    }
  });
});
