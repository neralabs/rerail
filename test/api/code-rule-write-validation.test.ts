import { readFileSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createRerailApi } from "../../src/api/http.js";
import { operations, type RerailApiStores } from "../../src/api/operations.js";
import type { CodeRule } from "../../src/engine/index.js";
import { exportScopedRules, importScopedRules, SqliteRuleStore, ScopedRuleCatalog } from "../../src/engine/index.js";
import { MOCK_CODE_RULES } from "../../src/cli/index.js";

/** Zod accepts it; the engine rejects it because `$TYPO` names no capture of the query. */
const unboundMessage: CodeRule = {
  format: "rerail.policy/v1",
  id: "test:unbound-message",
  revision: 1,
  mode: "steer",
  clauses: [{
    id: "call",
    forbid: { ast: { language: "typescript", rule: { pattern: "foo($A)" } } },
    violation: { anchor: "witness.span", code: "X", message: "bad $TYPO" },
  }],
};
const ENGINE_REASON = /\/clauses\/0\/violation\/message: `\$TYPO` names no capture/;

let root: string;
const originalEnginePath = process.env.RERAIL_NATIVE_KERNEL_PATH;

beforeEach(async () => {
  root = await mkdtemp(join(tmpdir(), "rerail-policy-write-"));
});

afterEach(async () => {
  if (originalEnginePath === undefined) delete process.env.RERAIL_NATIVE_KERNEL_PATH;
  else process.env.RERAIL_NATIVE_KERNEL_PATH = originalEnginePath;
  await rm(root, { recursive: true, force: true });
});

function scopedCatalog(name = "catalog.sqlite"): ScopedRuleCatalog {
  return ScopedRuleCatalog.open(join(root, name), { repositoryId: `repo-${name}`, workspaceRoot: root });
}

describe("code rule writes compile in the engine", () => {
  it("rejects a rule the engine cannot compile at ScopedRuleCatalog.upsertCodeRule, with the engine's reason", () => {
    const catalog = scopedCatalog();
    try {
      expect(() => catalog.upsertCodeRule(unboundMessage)).toThrow(ENGINE_REASON);
      expect(() => catalog.upsertCodeRule(unboundMessage)).toThrow(/policy rule test:unbound-message does not compile/);
      expect(catalog.get(unboundMessage.id)).toBeUndefined();
      expect(catalog.upsertCodeRule(MOCK_CODE_RULES[1]!).rule).toEqual(MOCK_CODE_RULES[1]);
    } finally {
      catalog.close();
    }
  });

  it("rejects the same rule at SqliteRuleStore.upsertCodeRule", () => {
    const store = new SqliteRuleStore(join(root, "rules.sqlite"));
    try {
      expect(() => store.upsertCodeRule(unboundMessage)).toThrow(ENGINE_REASON);
      expect(store.get(unboundMessage.id)).toBeUndefined();
      expect(store.upsertCodeRule(MOCK_CODE_RULES[1]!).rule).toEqual(MOCK_CODE_RULES[1]);
    } finally {
      store.close();
    }
  });

  it("refuses to store a code rule it cannot validate when the native engine is unavailable", () => {
    const catalog = scopedCatalog();
    try {
      process.env.RERAIL_NATIVE_KERNEL_PATH = join(root, "missing.node");
      expect(() => catalog.upsertCodeRule(MOCK_CODE_RULES[0]!)).toThrow(/cannot validate policy rule mock-no-console-log: native kernel unavailable/);
      expect(catalog.get(MOCK_CODE_RULES[0]!.id)).toBeUndefined();
    } finally {
      catalog.close();
    }
  });

  it("answers rules_upsert with 400 invalid-rule and the engine's reason", async () => {
    const catalog = scopedCatalog();
    const stores = { rules: () => catalog } as unknown as RerailApiStores;
    const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    try {
      const rejected = await app.request(`/api/rules/${unboundMessage.id}`, {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ kind: "file", rule: unboundMessage }),
      });
      expect(rejected.status).toBe(400);
      const body = await rejected.json() as { error: string; message: string };
      expect(body.error).toBe("invalid-rule");
      expect(body.message).toMatch(ENGINE_REASON);
      expect(catalog.get(unboundMessage.id)).toBeUndefined();

      process.env.RERAIL_NATIVE_KERNEL_PATH = join(root, "missing.node");
      const unavailable = await app.request(`/api/rules/${MOCK_CODE_RULES[0]!.id}`, {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ kind: "file", rule: MOCK_CODE_RULES[0] }),
      });
      expect(unavailable.status).toBe(503);
      expect(await unavailable.json()).toMatchObject({ error: "kernel-unavailable" });
    } finally {
      catalog.close();
    }
  });

  it("rejects a whole import that contains a rule the engine cannot compile, naming it", () => {
    const source = scopedCatalog("source.sqlite");
    const dumpPath = join(root, "rules.json");
    try {
      source.upsertCodeRule(MOCK_CODE_RULES[0]!);
      source.upsertCodeRule(MOCK_CODE_RULES[1]!);
      exportScopedRules(source, dumpPath);
    } finally {
      source.close();
    }
    const dump = JSON.parse(readFileSync(dumpPath, "utf8")) as { rules: Array<{ rule: unknown }> };
    dump.rules[1]!.rule = unboundMessage;
    writeFileSync(dumpPath, JSON.stringify(dump));

    const destination = scopedCatalog("destination.sqlite");
    try {
      let error: unknown;
      try {
        importScopedRules(destination, dumpPath);
      } catch (caught) {
        error = caught;
      }
      expect((error as Error).message).toMatch(/rules\[1\] \(test:unbound-message\)/);
      expect((error as Error).message).toMatch(ENGINE_REASON);
      expect(destination.list()).toEqual([]);
    } finally {
      destination.close();
    }
  });
});
