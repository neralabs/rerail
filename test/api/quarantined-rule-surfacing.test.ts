import { execFileSync } from "node:child_process";
import { realpathSync } from "node:fs";
import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { eq } from "drizzle-orm";
import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from "vitest";
import { createRerailApi } from "../../src/api/http.js";
import { operations } from "../../src/api/operations.js";
import { DatabaseApiStores } from "../../src/api/stores.js";
import {
  loadBundledCatalog,
  openRerailDatabase,
  upsertLibraryRule,
  NativeCodeRuleEnforcer,
  type AuditRecorder,
  type AuditInput,
  type AuditResult,
  CheckoutCatalogs,
  codeRuleSchema,
  type CodeRule,
  type RuleDocument,
  libraryRules,
} from "../../src/engine/index.js";
import { setUpTestRepository } from "../support/repository-rules.js";

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

/** A set-up repository whose library holds `test:kept` and `test:broken`, whose row no longer validates. */
function repositoryWithBrokenRow(): { checkout: string; databasePath: string } {
  const checkout = realpathSync(root);
  execFileSync("git", ["init", "--quiet"], { cwd: checkout });
  const databasePath = join(checkout, ".rerail", "rerail.sqlite");
  setUpTestRepository(checkout, { library: [{ kind: "code", rule: codeRule("test:kept") }, { kind: "code", rule: codeRule("test:broken") }] }, databasePath);
  const database = openRerailDatabase(databasePath);
  try {
    const broken = { kind: "code", rule: { ...codeRule("test:broken"), clauses: [{ id: "x" }] } } as unknown as RuleDocument;
    database.update(libraryRules).set({ document: broken }).where(eq(libraryRules.ruleId, "test:broken")).run();
  } finally {
    database.$client.close();
  }
  return { checkout, databasePath };
}

const quarantineFailure = {
  ruleId: "test:broken",
  clauseId: "catalog",
  code: "adapter-error",
  stage: "parse",
  message: expect.stringMatching(/^policy rule test:broken \(library\) is skipped: clauses\/0/u),
};

describe("quarantined code rule rows", () => {
  it("are listed by the compiled catalog for the load that skipped them", () => {
    const { checkout, databasePath } = repositoryWithBrokenRow();
    const catalogs = new CheckoutCatalogs(databasePath);
    try {
      const catalog = catalogs.catalog(checkout);
      expect(catalog.snapshot().codeRules.map(entry => entry.id)).toEqual(["test:kept"]);
      expect(catalog.quarantined()).toEqual([
        { ruleId: "test:broken", kind: "code", owner: "library", reason: expect.stringMatching(/clauses/u) },
      ]);
    } finally {
      catalogs.close();
    }
  });

  it("surface as one adapter-error per rule per catalog load, and clear once the row is fixed", async () => {
    const { checkout, databasePath } = repositoryWithBrokenRow();
    await mkdir(join(checkout, "api"));
    const catalogs = new CheckoutCatalogs(databasePath);
    const { recorder, audits } = recordingAudits();
    const enforcer = new NativeCodeRuleEnforcer(catalogs.catalog(checkout), {
      workspaceRoot: checkout,
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

      // Fixing a library rule writes a new revision, which the catalog sees as a configuration change.
      upsertLibraryRule(catalogs.database, loadBundledCatalog(), { ...codeRule("test:broken"), revision: 2 }, "code");
      await enforcer.auditGeneratedEdits("session", {
        revision: 3, checkpoints: ["edit"], edits: [{ path: "api/a.py", language: "python", generatedText: "NEEDLE = 3\n" }],
      });
      await enforcer.settle();
      expect(enforcer.snapshotStatus().failures).toEqual([]);
      expect(reports()).toHaveLength(1);
    } finally {
      await enforcer.close();
      catalogs.close();
    }
  });

  it("are listed beside the library rules by library_rules_list", async () => {
    const databasePath = join(root, "rerail.sqlite");
    const stores = new DatabaseApiStores(databasePath, {
      health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
    });
    try {
      upsertLibraryRule(stores.database(), loadBundledCatalog(), codeRule("test:kept"), "code");
      upsertLibraryRule(stores.database(), loadBundledCatalog(), codeRule("test:broken"), "code");
      const raw = new Database(databasePath);
      raw.prepare("UPDATE library_rules SET document_json = json_set(document_json, '$.rule.clauses', json(?)) WHERE rule_id = ?")
        .run(JSON.stringify([{ id: "x" }]), "test:broken");
      raw.close();
      const app = createRerailApi(stores, operations, { version: "0.0.0-test" });

      const listed = await (await app.request("/api/library-rules")).json() as { rules: { rule: { id: string } }[]; quarantined: unknown[] };
      expect(listed.rules.map(stored => stored.rule.id)).toEqual(["test:kept"]);
      expect(listed.quarantined).toEqual([{ ruleId: "test:broken", kind: "code", owner: "library", reason: expect.stringMatching(/clauses/u) }]);
      const conversationOnly = await (await app.request("/api/library-rules?kind=conversation")).json() as { quarantined: unknown[] };
      expect(conversationOnly.quarantined).toEqual([]);
    } finally {
      stores.close();
    }
  });
});
