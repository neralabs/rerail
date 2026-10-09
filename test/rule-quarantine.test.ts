import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { sql } from "drizzle-orm";
import { afterEach, describe, expect, it, vi } from "vitest";
import { openRerailDatabase } from "../src/database.js";
import { bootstrapCodeRuleDslCatalog } from "../src/code-rule-dsl-catalog.js";
import { codeRuleSchema, type CodeRule } from "../src/code-rule-schema.js";
import { SqliteRuleStore } from "../src/rule-store.js";
import { RerailRuntime } from "../src/runtime.js";
import { ScopedRuleCatalog } from "../src/scoped-rule-catalog.js";
import { ruleDocumentDigest } from "../src/stable-json.js";
import { readStoredCodeRule } from "../src/rule-quarantine.js";

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function temporaryRoot(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-stored-rules-"));
  temporaryDirectories.push(root);
  return root;
}

const repository = { repositoryId: "stored-repo", workspaceRoot: "/stored-repo" };

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

/** Overwrite a catalog row's stored document behind the schema, as another process could have left it. */
function writeCatalogDocument(path: string, ruleId: string, rule: unknown): void {
  const database = openRerailDatabase(path);
  try {
    database.run(sql`UPDATE catalog_rules SET document_json = ${JSON.stringify({ kind: "code", rule })} WHERE rule_id = ${ruleId}`);
  } finally {
    database.$client.close();
  }
}

/** Store each document in a catalog: insert a clean placeholder, then swap the raw document in. */
function catalogWithRows(path: string, rules: readonly Record<string, unknown>[], sourceDigest: (rule: unknown) => string | null = () => null): void {
  const catalog = ScopedRuleCatalog.open(path, repository);
  try {
    for (const rule of rules) catalog.upsertCodeRule(cleanRule(String(rule.id)), { sourceDigest: sourceDigest(rule), origin: "rerail" });
  } finally {
    catalog.close();
  }
  for (const rule of rules) writeCatalogDocument(path, String(rule.id), rule);
}

const codeRuleRevision = (stored: { kind: string; rule: unknown } | undefined): number | undefined =>
  stored?.kind === "code" ? (stored.rule as CodeRule).revision : undefined;

const rawDigest = (rule: unknown): string => ruleDocumentDigest({ kind: "code", rule: rule as CodeRule });

const bundledSeedRule = (): CodeRule => codeRuleSchema.parse({ ...SEED_RULE, revision: 3 });

describe("readStoredCodeRule", () => {
  it("returns a valid body parsed, and names the first schema issue of an invalid one", () => {
    expect(readStoredCodeRule(SEED_RULE)).toEqual({ ok: true, rule: codeRuleSchema.parse(SEED_RULE) });
    expect(readStoredCodeRule(GARBAGE_RULE)).toEqual({ ok: false, reason: expect.stringMatching(/^clauses: /u) });
    expect(readStoredCodeRule("not an object")).toEqual({ ok: false, reason: expect.any(String) });
  });
});

describe("reading stored rows", () => {
  it("skips and diagnoses a row that fails validation, and every other rule still loads", async () => {
    const path = join(await temporaryRoot(), "catalog.sqlite");
    catalogWithRows(path, [SEED_RULE, GARBAGE_RULE]);
    const catalog = ScopedRuleCatalog.open(path, repository);
    const warnings = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    try {
      expect(catalog.list().map(stored => stored.rule.id)).toEqual([SEED_RULE.id]);
      expect(catalog.list().map(stored => stored.rule.id)).toEqual([SEED_RULE.id]);
      const skipped = warnings.mock.calls.map(([text]) => String(text)).filter(text => text.includes("rule garbage"));
      expect(skipped).toEqual([expect.stringMatching(/^rerail: skipping code rule garbage \(project:[^)]+\) in .*catalog\.sqlite: clauses: /u)]);
      expect(catalog.get("garbage")).toBeUndefined();
      expect(catalog.quarantined()).toEqual([
        { ruleId: "garbage", kind: "code", owner: `project:${catalog.context().projectId}`, reason: expect.stringMatching(/clauses/u) },
      ]);
    } finally {
      warnings.mockRestore();
      catalog.close();
    }
    const compiled = bootstrapCodeRuleDslCatalog({ databasePath: path, store: ScopedRuleCatalog.open(path, repository) });
    try {
      expect(compiled.codeRules().map(entry => entry.id)).toEqual([SEED_RULE.id]);
    } finally {
      compiled.close();
    }
  });

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

  it("constructs a RerailRuntime over a catalog holding an invalid row", async () => {
    const root = await temporaryRoot();
    const priorHome = process.env.RERAIL_HOME;
    process.env.RERAIL_HOME = join(root, "home");
    const workspaceRoot = join(root, "workspace");
    const repositoryContext = { repositoryId: "stored-runtime", workspaceRoot };
    try {
      const path = join(root, "home", "rerail.sqlite");
      const catalog = ScopedRuleCatalog.open(path, repositoryContext);
      try {
        for (const rule of [SEED_RULE, GARBAGE_RULE]) catalog.upsertCodeRule(cleanRule(rule.id));
      } finally {
        catalog.close();
      }
      writeCatalogDocument(path, SEED_RULE.id, SEED_RULE);
      writeCatalogDocument(path, GARBAGE_RULE.id, GARBAGE_RULE);
      const runtime = new RerailRuntime({
        autostart: false,
        workspaceRoot,
        repositoryContext,
        storageRoot: join(root, ".rerail", "sessions"),
        embedder: {
          fingerprint: () => ({ modelId: "test", revision: "1", dimensions: 2, weightsSha256: "test", tokenizerSha256: "test", kernelFormatVersion: 1 }),
          embed: async texts => texts.map(() => new Float32Array([1, 0])),
        },
      });
      await runtime.dispose();
      const reopened = ScopedRuleCatalog.open(path, repositoryContext);
      try {
        expect(reopened.list().map(stored => stored.rule.id)).toEqual(expect.arrayContaining([SEED_RULE.id]));
        expect(reopened.quarantined()).toEqual(expect.arrayContaining([
          expect.objectContaining({ ruleId: "garbage", kind: "code", owner: `project:${reopened.context().projectId}` }),
        ]));
        expect(reopened.get("garbage")).toBeUndefined();
      } finally {
        reopened.close();
      }
    } finally {
      if (priorHome === undefined) delete process.env.RERAIL_HOME;
      else process.env.RERAIL_HOME = priorHome;
    }
  });
});

describe("seeding over stored rows", () => {
  it("refreshes an untouched bundled row, and keeps one the user edited or one with no bundled provenance", async () => {
    const path = join(await temporaryRoot(), "catalog.sqlite");
    catalogWithRows(path, [SEED_RULE], rawDigest);
    const catalog = ScopedRuleCatalog.open(path, repository);
    try {
      expect(catalog.seed([{ kind: "code", rule: bundledSeedRule() }])).toEqual({ inserted: 0, refreshed: 1, kept: 0 });
      expect(codeRuleRevision(catalog.get(SEED_RULE.id))).toBe(3);
    } finally {
      catalog.close();
    }

    const editedPath = join(await temporaryRoot(), "catalog.sqlite");
    const other = { ...cleanRule("unprovenanced"), revision: 1 };
    catalogWithRows(editedPath, [SEED_RULE, other], rule => (rule === SEED_RULE ? "sha256:edited-since-seed" : null));
    const edited = ScopedRuleCatalog.open(editedPath, repository);
    try {
      const report = edited.seed([
        { kind: "code", rule: bundledSeedRule() },
        { kind: "code", rule: codeRuleSchema.parse({ ...other, revision: 2 }) },
      ]);
      expect(report).toEqual({ inserted: 0, refreshed: 0, kept: 2 });
      expect(codeRuleRevision(edited.get(SEED_RULE.id))).toBe(2);
    } finally {
      edited.close();
    }
  });

  it("replaces an untouched bundled row the schema now rejects, but never an invalid row the user owns", async () => {
    const path = join(await temporaryRoot(), "catalog.sqlite");
    const invalid = { ...SEED_RULE, clauses: [] };
    catalogWithRows(path, [invalid], rawDigest);
    const catalog = ScopedRuleCatalog.open(path, repository);
    try {
      expect(catalog.seed([{ kind: "code", rule: bundledSeedRule() }])).toEqual({ inserted: 0, refreshed: 1, kept: 0 });
      expect(codeRuleRevision(catalog.get(SEED_RULE.id))).toBe(3);
    } finally {
      catalog.close();
    }

    const userPath = join(await temporaryRoot(), "catalog.sqlite");
    catalogWithRows(userPath, [invalid]);
    const userCatalog = ScopedRuleCatalog.open(userPath, repository);
    try {
      expect(userCatalog.seed([{ kind: "code", rule: bundledSeedRule() }])).toEqual({ inserted: 0, refreshed: 0, kept: 1 });
      expect(userCatalog.quarantined()).toHaveLength(1);
    } finally {
      userCatalog.close();
    }
  });

  it("applies the same rules in the per-project rule store", async () => {
    const path = join(await temporaryRoot(), "store.sqlite");
    const store = new SqliteRuleStore(path);
    try {
      store.upsertCodeRule(cleanRule(SEED_RULE.id), { sourceDigest: rawDigest(SEED_RULE) });
    } finally {
      store.close();
    }
    const database = openRerailDatabase(path);
    try {
      database.run(sql`UPDATE rules SET clauses_json = ${JSON.stringify(SEED_RULE.clauses)}, revision = 2,
        description = ${SEED_RULE.description}, scope_json = ${JSON.stringify(SEED_RULE.scope)} WHERE id = ${SEED_RULE.id}`);
    } finally {
      database.$client.close();
    }
    const reopened = new SqliteRuleStore(path);
    try {
      expect(reopened.seed([{ kind: "code", rule: bundledSeedRule() }])).toEqual({ inserted: 0, refreshed: 1, kept: 0 });
      expect(codeRuleRevision(reopened.get(SEED_RULE.id))).toBe(3);
    } finally {
      reopened.close();
    }
  });
});
