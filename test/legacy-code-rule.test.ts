import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { sql } from "drizzle-orm";
import { afterEach, describe, expect, it, vi } from "vitest";
import { openRerailDatabase } from "../src/database.js";
import { normalizeLegacyCodeRule } from "../src/legacy-code-rule.js";
import { loadNativeBinding } from "../src/native.js";
import { bootstrapCodeRuleDslCatalog } from "../src/code-rule-dsl-catalog.js";
import { codeRuleSchema, type CodeRule } from "../src/code-rule-schema.js";
import { SqliteRuleStore } from "../src/rule-store.js";
import { RerailRuntime } from "../src/runtime.js";
import { ScopedRuleCatalog } from "../src/scoped-rule-catalog.js";
import { ruleDocumentDigest } from "../src/stable-json.js";

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function temporaryRoot(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-legacy-rules-"));
  temporaryDirectories.push(root);
  return root;
}

const repository = { repositoryId: "legacy-repo", workspaceRoot: "/legacy-repo" };

/** A `nera-data-layer` row as bundled before #2561: `forbid.ast.capture` was accepted and ignored. */
const LEGACY_DATA_LAYER_RULE = {
  format: "rerail.policy/v1",
  id: "nera-data-layer:connection_mode_split",
  revision: 1,
  mode: "steer",
  scope: { include: ["**/*.py"], exclude: [] },
  clauses: [
    {
      id: "nera-data-layer-connection-mode-split",
      violation: { code: "connection_mode_split", anchor: "witness.span", message: "Mode-specific connection surface enforcement" },
      forbid: {
        ast: {
          language: "python",
          rule: { any: [{ all: [{ kind: "import_statement" }, { regex: "\\bDatabaseConnection\\b" }] }] },
          capture: "witness",
        },
      },
    },
  ],
};

/** `MOCK_CODE_RULES[1]` as `rerail seed` wrote it before #2561. */
const LEGACY_SEED_RULE = {
  format: "rerail.policy/v1",
  id: "mock-lazy-query-shaping",
  revision: 2,
  mode: "steer",
  description: "Route reads must stay inside the lazy query plan; materializing before filtering defeats the ServerPlan budget.",
  scope: { include: ["api/nera_api/routes/**/*.py"], exclude: ["api/nera_api/routes/health.py"] },
  clauses: [
    {
      id: "list-of-query",
      forbid: { ast: { language: "python", capture: "CALL", rule: { pattern: "list($QUERY)" } } },
      violation: { anchor: "witness.span", code: "RERAIL-MOCK-002", message: "Query materialized before filters were applied." },
    },
  ],
};

/** Legacy violation fields: a free-form anchor, empty and non-empty `variables`, and `regex.captures` under `select_when`. */
const LEGACY_FINDING_RULE = {
  format: "rerail.policy/v1",
  id: "legacy-finding-fields",
  revision: 1,
  mode: "steer",
  clauses: [
    {
      id: "print-call",
      violation: { anchor: "subject.id", code: "NO_PRINT", message: "No print", variables: {} },
      forbid: { regex: { source: "file.content", pattern: "\\bprint\\(", captures: { CALL: 0 } } },
    },
    {
      id: "gated",
      violation: { anchor: "witness.span", code: "GATED", message: "Gated", variables: { level: "high" } },
      forbid: {
        select_when: {
          select: { regex: { source: "file.content", pattern: "\\bprint\\(", captures: [] } },
          exists: { ast: { language: "python", rule: { pattern: "import os" }, capture: "IMPORT" } },
        },
      },
    },
  ],
};

const LEGACY_RULES = [LEGACY_DATA_LAYER_RULE, LEGACY_SEED_RULE, LEGACY_FINDING_RULE];

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

/** Overwrite a catalog row's stored document behind the schema, as an older Rerail version would have left it. */
function writeCatalogDocument(path: string, ruleId: string, rule: unknown): void {
  const database = openRerailDatabase(path);
  try {
    database.run(sql`UPDATE catalog_rules SET document_json = ${JSON.stringify({ kind: "file", rule })} WHERE rule_id = ${ruleId}`);
  } finally {
    database.$client.close();
  }
}

function writeLegacyStoreChecks(path: string, ruleId: string, checks: unknown): void {
  const database = openRerailDatabase(path);
  try {
    database.run(sql`UPDATE rules SET clauses_json = ${JSON.stringify(checks)} WHERE id = ${ruleId}`);
  } finally {
    database.$client.close();
  }
}

/** Store each legacy document in a catalog: insert a clean placeholder, then swap the raw document in. */
function catalogWithLegacyRows(path: string, rules: readonly Record<string, unknown>[], sourceDigest: (rule: unknown) => string | null = () => null): void {
  const catalog = ScopedRuleCatalog.open(path, repository);
  try {
    for (const rule of rules) catalog.upsertCodeRule(cleanRule(String(rule.id)), { sourceDigest: sourceDigest(rule), origin: "rerail" });
  } finally {
    catalog.close();
  }
  for (const rule of rules) writeCatalogDocument(path, String(rule.id), rule);
}

const codeRuleRevision = (stored: { kind: string; rule: unknown } | undefined): number | undefined =>
  stored?.kind === "file" ? (stored.rule as CodeRule).revision : undefined;

const rawDigest = (rule: unknown): string => ruleDocumentDigest({ kind: "file", rule: rule as CodeRule });

describe("normalizeLegacyCodeRule", () => {
  it("drops ast.capture and regex.captures in every query position, maps a free-form anchor to witness.span, and drops variables", () => {
    const before = structuredClone(LEGACY_FINDING_RULE);
    const { rule, changes } = normalizeLegacyCodeRule(LEGACY_FINDING_RULE);
    expect(LEGACY_FINDING_RULE).toEqual(before);
    expect(codeRuleSchema.safeParse(rule).success).toBe(true);
    expect(rule).toEqual({
      ...LEGACY_FINDING_RULE,
      clauses: [
        {
          id: "print-call",
          violation: { anchor: "witness.span", code: "NO_PRINT", message: "No print" },
          forbid: { regex: { source: "file.content", pattern: "\\bprint\\(" } },
        },
        {
          id: "gated",
          violation: { anchor: "witness.span", code: "GATED", message: "Gated" },
          forbid: {
            select_when: {
              select: { regex: { source: "file.content", pattern: "\\bprint\\(" } },
              exists: { ast: { language: "python", rule: { pattern: "import os" } } },
            },
          },
        },
      ],
    });
    expect(changes).toEqual([
      "clauses/0/violation/anchor: subject.id -> witness.span",
      "clauses/0/violation/variables: dropped",
      "clauses/0/forbid/regex/captures: dropped",
      "clauses/1/violation/variables: dropped",
      "clauses/1/forbid/select_when/select/regex/captures: dropped",
      "clauses/1/forbid/select_when/exists/ast/capture: dropped",
    ]);
  });

  it("returns a current document unchanged, and leaves anything it does not recognize for the schema to reject", () => {
    const current = cleanRule("current");
    expect(normalizeLegacyCodeRule(current)).toEqual({ rule: current, changes: [] });
    expect(normalizeLegacyCodeRule({ id: "garbage" })).toEqual({ rule: { id: "garbage" }, changes: [] });
    expect(normalizeLegacyCodeRule("not an object")).toEqual({ rule: "not an object", changes: [] });
  });

  it("anchors a fact or llm clause's non-span anchor at witness.span, the only anchor the engine accepts there", () => {
    const { rule, changes } = normalizeLegacyCodeRule({
      format: "rerail.policy/v1",
      id: "legacy-llm",
      revision: 1,
      mode: "log",
      clauses: [{ id: "rubric", violation: { anchor: "capture.CALL", code: "LLM", message: "m" }, llm: { instruction: "x" } }],
    });
    expect((rule as CodeRule).clauses[0]!.violation?.anchor).toBe("witness.span");
    expect(changes).toEqual(["clauses/0/violation/anchor: capture.CALL -> witness.span"]);
  });
});

describe("reading legacy rows", () => {
  it("reads legacy data-layer, seed-mock, and finding-field rows through the scoped catalog without rewriting them", async () => {
    const path = join(await temporaryRoot(), "catalog.sqlite");
    catalogWithLegacyRows(path, LEGACY_RULES);
    const catalog = ScopedRuleCatalog.open(path, repository);
    try {
      const listed = catalog.list({ kind: "file" });
      expect(listed.map(stored => stored.rule.id).sort()).toEqual(LEGACY_RULES.map(rule => rule.id).sort());
      for (const stored of listed) expect(codeRuleSchema.safeParse(stored.rule).success).toBe(true);
      expect(catalog.get(LEGACY_SEED_RULE.id)?.rule).toEqual(normalizeLegacyCodeRule(LEGACY_SEED_RULE).rule);
      expect(catalog.codeRules()).toHaveLength(LEGACY_RULES.length);
      expect(catalog.quarantined()).toEqual([]);
    } finally {
      catalog.close();
    }
    const database = openRerailDatabase(path);
    try {
      const stored = database.all<{ document_json: string }>(sql`SELECT document_json FROM catalog_rules WHERE rule_id = ${LEGACY_SEED_RULE.id}`);
      expect(JSON.parse(stored[0]!.document_json).rule).toEqual(LEGACY_SEED_RULE);
    } finally {
      database.$client.close();
    }
  });

  it("reads legacy rows through the per-project rule store, and migrates them into the catalog", async () => {
    const root = await temporaryRoot();
    const legacyPath = join(root, ".rerail", "transcripts.sqlite");
    const store = new SqliteRuleStore(legacyPath);
    try {
      for (const rule of LEGACY_RULES) store.upsertCodeRule(cleanRule(rule.id));
    } finally {
      store.close();
    }
    for (const rule of LEGACY_RULES) writeLegacyStoreChecks(legacyPath, rule.id, rule.clauses);
    const reopened = new SqliteRuleStore(legacyPath);
    try {
      expect(reopened.codeRules().map(rule => rule.id).sort()).toEqual(LEGACY_RULES.map(rule => rule.id).sort());
      const normalized = normalizeLegacyCodeRule(LEGACY_DATA_LAYER_RULE).rule as CodeRule;
      expect((reopened.get(LEGACY_DATA_LAYER_RULE.id)?.rule as CodeRule).clauses).toEqual(normalized.clauses);
      expect(reopened.quarantined()).toEqual([]);
    } finally {
      reopened.close();
    }
    const catalog = ScopedRuleCatalog.open(join(root, "catalog.sqlite"), repository);
    try {
      expect(catalog.migrateLegacyRules(legacyPath)).toBe(LEGACY_RULES.length);
      expect(catalog.codeRules()).toHaveLength(LEGACY_RULES.length);
    } finally {
      catalog.close();
    }
  });

  it("compiles every normalized legacy rule, and the capture-bearing rule reports its witness span as before", async () => {
    const path = join(await temporaryRoot(), "catalog.sqlite");
    catalogWithLegacyRows(path, LEGACY_RULES);
    const catalog = bootstrapCodeRuleDslCatalog({ databasePath: path, store: ScopedRuleCatalog.open(path, repository) });
    try {
      expect(catalog.codeRules().map(entry => entry.id).sort()).toEqual(LEGACY_RULES.map(rule => rule.id).sort());
      const source = "rows = list(query)\n";
      const input = JSON.stringify({
        facts: [],
        files: [{ language: "python", path: "api/nera_api/routes/users.py", source }],
        format: "rerail.repository-snapshot/v1",
        revision: 0,
      });
      const result = JSON.parse(catalog.codeRule(LEGACY_SEED_RULE.id).evaluate(input, "report")) as {
        violations: { code: string; span: { startByte: number; endByte: number } }[];
      };
      const start = source.indexOf("list(query)");
      expect(result.violations).toMatchObject([{ code: "RERAIL-MOCK-002", span: { startByte: start, endByte: start + "list(query)".length } }]);
    } finally {
      catalog.close();
    }
    expect(loadNativeBinding().CodeRuleDslRuntime).toBeDefined();
  });

  it("skips and diagnoses a row that still fails validation, and every other rule still loads", async () => {
    const path = join(await temporaryRoot(), "catalog.sqlite");
    catalogWithLegacyRows(path, [LEGACY_SEED_RULE, { format: "rerail.policy/v1", id: "garbage", revision: 1, mode: "steer" }]);
    const catalog = ScopedRuleCatalog.open(path, repository);
    const warnings = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    try {
      expect(catalog.list().map(stored => stored.rule.id)).toEqual([LEGACY_SEED_RULE.id]);
      expect(catalog.list().map(stored => stored.rule.id)).toEqual([LEGACY_SEED_RULE.id]);
      const skipped = warnings.mock.calls.map(([text]) => String(text)).filter(text => text.includes("rule garbage"));
      expect(skipped).toEqual([expect.stringMatching(/^rerail: skipping file rule garbage \(project:[^)]+\) in .*catalog\.sqlite: clauses: /u)]);
      expect(catalog.get("garbage")).toBeUndefined();
      expect(catalog.quarantined()).toEqual([
        { ruleId: "garbage", kind: "file", owner: `project:${catalog.context().projectId}`, reason: expect.stringMatching(/clauses/u) },
      ]);
    } finally {
      warnings.mockRestore();
      catalog.close();
    }
    const compiled = bootstrapCodeRuleDslCatalog({ databasePath: path, store: ScopedRuleCatalog.open(path, repository) });
    try {
      expect(compiled.codeRules().map(entry => entry.id)).toEqual([LEGACY_SEED_RULE.id]);
    } finally {
      compiled.close();
    }
  });

  it("skips and diagnoses an invalid row in the per-project rule store", async () => {
    const path = join(await temporaryRoot(), ".rerail", "transcripts.sqlite");
    const store = new SqliteRuleStore(path);
    try {
      store.upsertCodeRule(cleanRule("kept"));
      store.upsertCodeRule(cleanRule("broken"));
    } finally {
      store.close();
    }
    writeLegacyStoreChecks(path, "broken", [{ id: "x" }]);
    const reopened = new SqliteRuleStore(path);
    try {
      expect(reopened.list().map(stored => stored.rule.id)).toEqual(["kept"]);
      expect(reopened.get("broken")).toBeUndefined();
      expect(reopened.quarantined()).toMatchObject([{ ruleId: "broken", kind: "file", owner: "repository" }]);
    } finally {
      reopened.close();
    }
  });

  it("constructs a RerailRuntime over a catalog holding legacy and invalid rows", async () => {
    const root = await temporaryRoot();
    const priorHome = process.env.RERAIL_HOME;
    process.env.RERAIL_HOME = join(root, "home");
    const workspaceRoot = join(root, "workspace");
    const repositoryContext = { repositoryId: "legacy-runtime", workspaceRoot };
    try {
      const path = join(root, "home", "rerail.sqlite");
      const catalog = ScopedRuleCatalog.open(path, repositoryContext);
      try {
        for (const rule of [...LEGACY_RULES, { id: "garbage" }]) catalog.upsertCodeRule(cleanRule(String(rule.id)));
      } finally {
        catalog.close();
      }
      for (const rule of LEGACY_RULES) writeCatalogDocument(path, rule.id, rule);
      writeCatalogDocument(path, "garbage", { format: "rerail.policy/v1", id: "garbage", revision: 1 , mode: "steer"});
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
        expect(reopened.list().map(stored => stored.rule.id)).toEqual(expect.arrayContaining(LEGACY_RULES.map(rule => String(rule.id))));
        expect(reopened.quarantined()).toEqual(expect.arrayContaining([
          expect.objectContaining({ ruleId: "garbage", kind: "file", owner: `project:${reopened.context().projectId}` }),
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

describe("seeding over legacy rows", () => {
  const bundledSeedRule = (): CodeRule => codeRuleSchema.parse({ ...normalizeLegacyCodeRule(LEGACY_SEED_RULE).rule as object, revision: 3 });

  it("refreshes an untouched legacy bundled row: its digest is judged on the stored document, not the normalized one", async () => {
    const path = join(await temporaryRoot(), "catalog.sqlite");
    catalogWithLegacyRows(path, [LEGACY_SEED_RULE], rawDigest);
    const catalog = ScopedRuleCatalog.open(path, repository);
    try {
      expect(catalog.seed([{ kind: "file", rule: bundledSeedRule() }])).toEqual({ inserted: 0, refreshed: 1, kept: 0 });
      expect(codeRuleRevision(catalog.get(LEGACY_SEED_RULE.id))).toBe(3);
    } finally {
      catalog.close();
    }
  });

  it("keeps a legacy row the user edited, and a legacy row with no bundled provenance", async () => {
    const path = join(await temporaryRoot(), "catalog.sqlite");
    catalogWithLegacyRows(path, [LEGACY_SEED_RULE, LEGACY_DATA_LAYER_RULE], rule => (rule === LEGACY_SEED_RULE ? "sha256:edited-since-seed" : null));
    const catalog = ScopedRuleCatalog.open(path, repository);
    try {
      const report = catalog.seed([
        { kind: "file", rule: bundledSeedRule() },
        { kind: "file", rule: codeRuleSchema.parse({ ...normalizeLegacyCodeRule(LEGACY_DATA_LAYER_RULE).rule as object, revision: 2 }) },
      ]);
      expect(report).toEqual({ inserted: 0, refreshed: 0, kept: 2 });
      expect(codeRuleRevision(catalog.get(LEGACY_SEED_RULE.id))).toBe(2);
    } finally {
      catalog.close();
    }
  });

  it("replaces an untouched bundled row the schema now rejects, but never an invalid row the user owns", async () => {
    const path = join(await temporaryRoot(), "catalog.sqlite");
    const invalid = { ...LEGACY_SEED_RULE, clauses: [] };
    catalogWithLegacyRows(path, [invalid], rawDigest);
    const catalog = ScopedRuleCatalog.open(path, repository);
    try {
      expect(catalog.seed([{ kind: "file", rule: bundledSeedRule() }])).toEqual({ inserted: 0, refreshed: 1, kept: 0 });
      expect(codeRuleRevision(catalog.get(LEGACY_SEED_RULE.id))).toBe(3);
    } finally {
      catalog.close();
    }

    const userPath = join(await temporaryRoot(), "catalog.sqlite");
    catalogWithLegacyRows(userPath, [invalid]);
    const userCatalog = ScopedRuleCatalog.open(userPath, repository);
    try {
      expect(userCatalog.seed([{ kind: "file", rule: bundledSeedRule() }])).toEqual({ inserted: 0, refreshed: 0, kept: 1 });
      expect(userCatalog.quarantined()).toHaveLength(1);
    } finally {
      userCatalog.close();
    }
  });

  it("applies the same rules in the per-project rule store", async () => {
    const path = join(await temporaryRoot(), ".rerail", "transcripts.sqlite");
    const store = new SqliteRuleStore(path);
    try {
      store.upsertCodeRule(cleanRule(LEGACY_SEED_RULE.id), { sourceDigest: rawDigest(LEGACY_SEED_RULE) });
    } finally {
      store.close();
    }
    const database = openRerailDatabase(path);
    try {
      database.run(sql`UPDATE rules SET clauses_json = ${JSON.stringify(LEGACY_SEED_RULE.clauses)}, revision = 2,
        description = ${LEGACY_SEED_RULE.description}, scope_json = ${JSON.stringify(LEGACY_SEED_RULE.scope)} WHERE id = ${LEGACY_SEED_RULE.id}`);
    } finally {
      database.$client.close();
    }
    const reopened = new SqliteRuleStore(path);
    try {
      expect(reopened.seed([{ kind: "file", rule: bundledSeedRule() }])).toEqual({ inserted: 0, refreshed: 1, kept: 0 });
      expect(codeRuleRevision(reopened.get(LEGACY_SEED_RULE.id))).toBe(3);
    } finally {
      reopened.close();
    }
  });
});
