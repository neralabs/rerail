import {
  loadBundledCatalog,
  type CodeRule,
  openRerailDatabase,
  type RerailDatabase,
  exportRulesDump,
  importRulesDump,
  type RulesDump,
  SqliteRuleStore,
  getLibraryRule,
  listLibraryRules,
  upsertLibraryRule,
} from "../../src/engine/index.js";
import { createRerailApi } from "../../src/api/http.js";
import { operations } from "../../src/api/operations.js";
import { DatabaseApiStores } from "../../src/api/stores.js";
import { MOCK_CODE_RULES } from "../../src/cli/index.js";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

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

function library(name = "rerail.sqlite"): RerailDatabase {
  return openRerailDatabase(join(root, name));
}

describe("code rule writes compile in the engine", () => {
  it("rejects a rule the engine cannot compile at upsertLibraryRule, with the engine's reason", () => {
    const database = library();
    const bundle = loadBundledCatalog();
    try {
      expect(() => upsertLibraryRule(database, bundle, unboundMessage, "file")).toThrow(ENGINE_REASON);
      expect(() => upsertLibraryRule(database, bundle, unboundMessage, "file")).toThrow(/policy rule test:unbound-message does not compile/);
      expect(getLibraryRule(database, unboundMessage.id)).toBeUndefined();
      expect(upsertLibraryRule(database, bundle, MOCK_CODE_RULES[1]!, "file").document.rule).toEqual(MOCK_CODE_RULES[1]);
    } finally {
      database.$client.close();
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
    const database = library();
    try {
      process.env.RERAIL_NATIVE_KERNEL_PATH = join(root, "missing.node");
      expect(() => upsertLibraryRule(database, loadBundledCatalog(), MOCK_CODE_RULES[0]!, "file"))
        .toThrow(/cannot validate policy rule mock-no-console-log: native kernel unavailable/);
      expect(getLibraryRule(database, MOCK_CODE_RULES[0]!.id)).toBeUndefined();
    } finally {
      database.$client.close();
    }
  });

  it("answers library_rules_upsert with 400 invalid-rule and the engine's reason", async () => {
    const databasePath = join(root, "rerail.sqlite");
    const stores = new DatabaseApiStores(databasePath, {
      health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
    });
    const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    try {
      const rejected = await app.request(`/api/library-rules/${unboundMessage.id}`, {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ kind: "file", rule: unboundMessage }),
      });
      expect(rejected.status).toBe(400);
      expect(await rejected.json()).toMatchObject({ error: "invalid-rule", message: expect.stringMatching(ENGINE_REASON) });
      expect(getLibraryRule(stores.database(), unboundMessage.id)).toBeUndefined();

      process.env.RERAIL_NATIVE_KERNEL_PATH = join(root, "missing.node");
      const unavailable = await app.request(`/api/library-rules/${MOCK_CODE_RULES[0]!.id}`, {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ kind: "file", rule: MOCK_CODE_RULES[0] }),
      });
      expect(unavailable.status).toBe(503);
      expect(await unavailable.json()).toMatchObject({ error: "kernel-unavailable" });
    } finally {
      stores.close();
    }
  });

  it("rejects a whole import that contains a rule the engine cannot compile, naming it", () => {
    const source = library("source.sqlite");
    const bundle = loadBundledCatalog();
    let dump: RulesDump;
    try {
      upsertLibraryRule(source, bundle, MOCK_CODE_RULES[0]!, "file");
      upsertLibraryRule(source, bundle, MOCK_CODE_RULES[1]!, "file");
      dump = exportRulesDump(source);
    } finally {
      source.$client.close();
    }
    const broken = { ...dump, library: dump.library.map((entry, index) => index === 1 ? { ...entry, rule: unboundMessage } : entry) };

    const destination = library("destination.sqlite");
    try {
      let error: unknown;
      try {
        importRulesDump(destination, bundle, broken);
      } catch (caught) {
        error = caught;
      }
      expect((error as Error).message).toMatch(/rules\[1\] \(test:unbound-message\)/);
      expect((error as Error).message).toMatch(ENGINE_REASON);
      expect(listLibraryRules(destination)).toEqual([]);
    } finally {
      destination.$client.close();
    }
  });
});
