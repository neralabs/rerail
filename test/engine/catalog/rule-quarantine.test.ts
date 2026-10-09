import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { sql } from "drizzle-orm";
import { afterEach, describe, expect, it } from "vitest";
import { createRerailApi, DatabaseApiStores, operations } from "../../../src/api/index.js";
import { loadBundledCatalog } from "../../../src/engine/catalog/bundled-rules.js";
import { openRerailDatabase } from "../../../src/engine/storage/database.js";
import { codeRuleSchema, type CodeRule } from "../../../src/engine/code-rule/code-rule-schema.js";
import { listLibraryRules, quarantinedLibraryRules, removeLibraryRule, upsertLibraryRule } from "../../../src/engine/catalog/rule-library.js";
import { SqliteRuleStore } from "../../../src/engine/catalog/rule-store.js";
import { readStoredCodeRule } from "../../../src/engine/catalog/rule-quarantine.js";
import { libraryRules, ruleCatalogIdentities } from "../../../src/engine/storage/schema.js";
import { conversation } from "../../support/rule-configuration-builders.js";

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function temporaryRoot(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-stored-rules-"));
  temporaryDirectories.push(root);
  return root;
}

/** `MOCK_CODE_RULES[1]` as `rerail seed` writes it. */
const SEED_RULE = {
  format: "rerail.policy/v1",
  id: "mock-lazy-query-shaping",
  revision: 2,
  mode: "steer",
  description: "Route reads must stay inside the lazy query plan; materializing before filtering defeats the ServerPlan budget.",
  scope: { include: ["api/app/routes/**/*.py"], exclude: ["api/app/routes/health.py"] },
  clauses: [
    {
      id: "list-of-query",
      forbid: { ast: { language: "python", rule: { pattern: "list($QUERY)" } } },
      violation: { anchor: "witness.span", code: "RERAIL-MOCK-002", message: "Query materialized before filters were applied." },
    },
  ],
};

/** A code rule with no clauses: valid JSON, rejected by the schema. */
const GARBAGE_RULE = { format: "rerail.policy/v1", id: "garbage", revision: 1, mode: "steer" };

function cleanRule(id: string): CodeRule {
  return codeRuleSchema.parse({
    format: "rerail.policy/v1",
    id,
    revision: 1,
    mode: "steer",
    clauses: [
      {
        id: `${id}:check`,
        violation: { anchor: "witness.span", code: "CLEAN", message: "clean" },
        forbid: { regex: { source: "file.content", pattern: "\\bclean\\(" } },
      },
    ],
  });
}

describe("readStoredCodeRule", () => {
  it("returns a valid body parsed, and names the first schema issue of an invalid one", () => {
    expect(readStoredCodeRule(SEED_RULE)).toEqual({ ok: true, rule: codeRuleSchema.parse(SEED_RULE) });
    expect(readStoredCodeRule(GARBAGE_RULE)).toEqual({ ok: false, reason: expect.stringMatching(/^clauses: /u) });
    expect(readStoredCodeRule("not an object")).toEqual({ ok: false, reason: expect.any(String) });
  });
});

describe("reading stored rows", () => {
  it("skips and diagnoses an invalid row in the per-project rule store", async () => {
    const path = join(await temporaryRoot(), "store.sqlite");
    const store = new SqliteRuleStore(path);
    try {
      store.upsertCodeRule(cleanRule("kept"));
      store.upsertCodeRule(cleanRule("broken"));
    } finally {
      store.close();
    }
    const database = openRerailDatabase(path);
    try {
      database.run(sql`UPDATE rules SET clauses_json = ${JSON.stringify([{ id: "x" }])} WHERE id = ${"broken"}`);
    } finally {
      database.$client.close();
    }
    const reopened = new SqliteRuleStore(path);
    try {
      expect(reopened.list().map(stored => stored.rule.id)).toEqual(["kept"]);
      expect(reopened.get("broken")).toBeUndefined();
      expect(reopened.quarantined()).toMatchObject([{ ruleId: "broken", kind: "code", owner: "repository" }]);
    } finally {
      reopened.close();
    }
  });
});

describe("a stored library row with an unknown rule kind", () => {
  // A database written before the kind checks can hold any kind, since the DDL does not add a check to a table that
  // already exists; the check is set aside here to write one.
  it("is quarantined as an unknown rule kind, under a kind the library_rules_list contract allows, and every other rule still loads", async () => {
    const path = join(await temporaryRoot(), "rerail.sqlite");
    const stores = new DatabaseApiStores(path, {
      health: () => ({ pid: process.pid, database: { path, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
    });
    try {
      const database = stores.database();
      const bundle = loadBundledCatalog();
      for (const id of ["kept", "code-file", "all-file"]) upsertLibraryRule(database, bundle, cleanRule(id), "code");
      upsertLibraryRule(database, bundle, conversation("conversation-file"), "conversation");
      database.run(sql`PRAGMA ignore_check_constraints = ON`);
      // The code rule kind's legacy kind name; all-file's stored document records it too.
      database.run(sql`UPDATE library_rules SET kind = 'file' WHERE rule_id IN ('code-file', 'all-file', 'conversation-file')`);
      database.run(sql`UPDATE library_rules SET document_json = json_set(document_json, '$.kind', kind) WHERE rule_id = 'all-file'`);
      database.run(sql`PRAGMA ignore_check_constraints = OFF`);
      const response = await createRerailApi(stores, operations, { version: "0.0.0-test" }).request("/api/library-rules");
      const listed: unknown = await response.json();
      const contract = operations.find(operation => operation.id === "library_rules_list")!.output;
      const reason = "unknown rule kind file";
      expect({
        status: response.status,
        rules: listLibraryRules(database).map(rule => rule.document.rule.id),
        quarantined: quarantinedLibraryRules(database),
        contractIssues: contract.safeParse(listed).error?.issues ?? [],
      }).toEqual({
        status: 200,
        rules: ["kept"],
        quarantined: [
          { ruleId: "all-file", kind: "code", owner: "library", reason },
          { ruleId: "code-file", kind: "code", owner: "library", reason },
          { ruleId: "conversation-file", kind: "conversation", owner: "library", reason },
        ],
        contractIssues: [],
      });
    } finally {
      stores.close();
    }
  });
});

describe("a rule id's stored kind", () => {
  // A database from before the library table keeps rule ids' kinds with no library row; the check is set aside here
  // to write the code rule kind's old name.
  it("binds no id when it has an unknown kind and no library row, and a known kind or a library row still binds it", async () => {
    const database = openRerailDatabase(join(await temporaryRoot(), "rerail.sqlite"));
    try {
      const bundle = loadBundledCatalog();
      upsertLibraryRule(database, bundle, cleanRule("stored-file"), "code");
      database.run(sql`PRAGMA ignore_check_constraints = ON`);
      // The code rule kind's legacy kind name with no library row, and a known kind with none.
      database.run(sql`INSERT INTO rule_catalog_identities (rule_id, kind) VALUES ('orphan-file', 'file'), ('orphan-conversation', 'conversation')`);
      for (const table of [libraryRules, ruleCatalogIdentities]) {
        // The legacy kind name on both of stored-file's rows.
        database.run(sql`UPDATE ${table} SET kind = 'file' WHERE rule_id = 'stored-file'`);
      }
      database.run(sql`PRAGMA ignore_check_constraints = OFF`);

      upsertLibraryRule(database, bundle, cleanRule("orphan-file"), "code");
      expect(() => upsertLibraryRule(database, bundle, cleanRule("orphan-conversation"), "code"))
        .toThrow("rule orphan-conversation exists with kind conversation; remove it before storing code");
      expect(() => upsertLibraryRule(database, bundle, cleanRule("stored-file"), "code"))
        .toThrow("rule stored-file exists with kind file; remove it before storing code");
      expect(removeLibraryRule(database, "stored-file")).toBe(true);
      upsertLibraryRule(database, bundle, cleanRule("stored-file"), "code");

      expect({
        kinds: database.$client.prepare("SELECT rule_id, kind FROM rule_catalog_identities ORDER BY rule_id").all(),
        rules: listLibraryRules(database).map(rule => rule.document.rule.id),
      }).toEqual({
        kinds: [
          { rule_id: "orphan-conversation", kind: "conversation" },
          { rule_id: "orphan-file", kind: "code" },
          { rule_id: "stored-file", kind: "code" },
        ],
        rules: ["orphan-file", "stored-file"],
      });
    } finally {
      database.$client.close();
    }
  });
});
