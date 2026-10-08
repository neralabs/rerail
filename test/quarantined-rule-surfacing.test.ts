import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from "vitest";
import { createRerailApi } from "../src/api/http.js";
import { operations, type RerailApiStores } from "../src/api/operations.js";
import { NativeCodeRuleEnforcer } from "../src/code-rule-enforcer.js";
import type { AuditRecorder, AuditInput, AuditResult } from "../src/audit.js";
import { bootstrapCodeRuleDslCatalog } from "../src/code-rule-dsl-catalog.js";
import { codeRuleSchema, type CodeRule } from "../src/code-rule-schema.js";
import { SqliteRuleStore } from "../src/rule-store.js";
import { ScopedRuleCatalog } from "../src/scoped-rule-catalog.js";

function codeRule(id: string): CodeRule {
  return codeRuleSchema.parse({
    format: "rerail.policy/v1", id, revision: 1,
    mode: "steer",
    scope: { include: ["api/**/*.py"], exclude: [] },
    clauses: [{ id: "needle", violation: { code: "NEEDLE", anchor: "witness.span", message: "No NEEDLE" }, forbid: { regex: { pattern: "NEEDLE", source: "file.content" } } }],
  });
}

function recordingAudits(): { recorder: AuditRecorder; audits: Map<string, AuditInput & { result?: AuditResult }> } {
  const audits = new Map<string, AuditInput & { result?: AuditResult }>();
  let sequence = 0;
  return {
    recorder: {
      start(input) {
        const auditId = `run-${++sequence}`;
        audits.set(auditId, { ...input });
        return auditId;
      },
      finish(auditId, result) { audits.get(auditId)!.result = result; },
      progress(auditId, result) { audits.get(auditId)!.result = { status: "completed", ...result }; },
      interrupt() {},
    },
    audits,
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
  const databasePath = join(root, ".rerail", "code-rules.sqlite");
  const store = new SqliteRuleStore(databasePath);
  store.upsertCodeRule(codeRule("test:kept"));
  store.upsertCodeRule(codeRule("test:broken"));
  store.close();
  const raw = new Database(databasePath);
  raw.prepare("UPDATE rules SET clauses_json = ? WHERE id = ?").run(JSON.stringify([{ id: "x" }]), "test:broken");
  raw.close();
  return databasePath;
}

const quarantineFailure = {
  ruleId: "test:broken",
  clauseId: "catalog",
  code: "adapter-error",
  stage: "parse",
  message: expect.stringMatching(/^policy rule test:broken \(repository\) is skipped: clauses\/0/u),
};

describe("quarantined code rule rows", () => {
  it("are listed by the compiled catalog for the load that skipped them", () => {
    const catalog = bootstrapCodeRuleDslCatalog({ databasePath: storeWithBrokenRow() });
    try {
      expect(catalog.codeRules().map(entry => entry.id)).toEqual(["test:kept"]);
      expect(catalog.quarantined()).toEqual([
        { ruleId: "test:broken", kind: "file", owner: "repository", reason: expect.stringMatching(/clauses/u) },
      ]);
    } finally {
      catalog.close();
    }
  });

  it("surface as one adapter-error per rule per catalog load, and clear once the row is fixed", async () => {
    const databasePath = storeWithBrokenRow();
    await mkdir(join(root, "api"));
    const catalog = bootstrapCodeRuleDslCatalog({ databasePath });
    const { recorder, audits } = recordingAudits();
    const enforcer = new NativeCodeRuleEnforcer(catalog, {
      workspaceRoot: root,
      steerDelivery: { requestSteer: async () => undefined },
      audits: recorder,
      mode: "log",
    });
    const reports = () => [...audits.values()].filter(audit => audit.result?.failures?.some(failure => failure.ruleId === "test:broken"));
    try {
      await enforcer.openWorkspace("session");
      expect(enforcer.snapshotStatus().failures).toEqual([quarantineFailure]);
      expect(reports()).toEqual([expect.objectContaining({ paths: [], result: { status: "failed", violations: [], failures: [quarantineFailure] } })]);

      for (const revision of [1, 2]) {
        await enforcer.auditGeneratedEdits("session", {
          revision, checkpoints: ["edit"], edits: [{ path: "api/a.py", language: "python", generatedText: `NEEDLE = ${revision}\n` }],
        });
        await enforcer.settle();
      }
      expect(reports()).toHaveLength(1);
      expect(enforcer.snapshotStatus().failures).toEqual([quarantineFailure]);

      const store = new SqliteRuleStore(databasePath);
      store.upsertCodeRule(codeRule("test:broken"));
      store.close();
      await enforcer.auditGeneratedEdits("session", {
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
      catalog.upsertCodeRule(codeRule("test:kept"));
      catalog.upsertCodeRule(codeRule("test:broken"));
      const raw = new Database(join(root, "catalog.sqlite"));
      raw.prepare("UPDATE catalog_rules SET document_json = json_set(document_json, '$.rule.clauses', json(?)) WHERE rule_id = ?")
        .run(JSON.stringify([{ id: "x" }]), "test:broken");
      raw.close();
      const app = createRerailApi({ rules: () => catalog } as unknown as RerailApiStores, operations, { version: "0.0.0-test" });

      const listed = await (await app.request("/api/rules")).json() as { rules: { rule: { id: string } }[]; quarantined: unknown[] };
      expect(listed.rules.map(stored => stored.rule.id)).toEqual(["test:kept"]);
      expect(listed.quarantined).toEqual([{
        ruleId: "test:broken",
        kind: "file",
        owner: `project:${catalog.context().projectId}`,
        reason: expect.stringMatching(/clauses/u),
      }]);
      const conversationOnly = await (await app.request("/api/rules?kind=conversation")).json() as { quarantined: unknown[] };
      expect(conversationOnly.quarantined).toEqual([]);
    } finally {
      catalog.close();
    }
  });
});
