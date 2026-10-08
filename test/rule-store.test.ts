import { readFileSync } from "node:fs";
import { readFile, mkdtemp, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { renderRerailSchemaModule, SCHEMA_SQL_PATH } from "../scripts/generate-schema-sql.js";
import { openRerailDatabase } from "../src/database.js";
import { codeRuleSchema, type CodeRule } from "../src/code-rule-schema.js";
import type { RuleMode } from "../src/rule-mode.js";
import { RERAIL_SCHEMA_COMPATIBILITY_FLOOR, RERAIL_SCHEMA_VERSION } from "../src/schema.js";
import { parseConversationRule, parseConversationRuleDocument, type ConversationRule } from "../src/rule-schema.js";
import { SqliteRuleStore, type RuleDocument } from "../src/rule-store.js";
import { ScopedRuleCatalog } from "../src/scoped-rule-catalog.js";
import { ruleDocumentDigest } from "../src/stable-json.js";
import { SqlitePresentationPreferenceStore } from "../src/presentation-preference-store.js";
import { MOCK_CONVERSATION_RULES, MOCK_CODE_RULES, MOCK_SESSION_ID, mockSessionId, seedRerailDatabase } from "../src/seed.js";
import { SqliteSessionReader, SqliteSessionStore } from "../src/session-store.js";
import { createV14Database, v14TableStatement } from "./support/v14-database.js";

const TOOL_CALL_RULES = parseConversationRuleDocument(JSON.parse(readFileSync(resolve("test/fixtures/tool-call-rules.json"), "utf8")));

/** The v11 `rules` table: the v14 DDL without the `mode` column and its CHECK. */
function rulesTableV11(): string {
  const current = v14TableStatement("rules");
  const column = "\n\t`mode` text NOT NULL,";
  const constraint = `\n\tCONSTRAINT "rules_mode" CHECK("rules"."mode" IN ('log', 'steer', 'block')),`;
  expect(current).toContain(column);
  expect(current).toContain(constraint);
  return current.replace(column, "").replace(constraint, "");
}

/** The v10 `rules` table: v11's with the one CHECK clause v11 relaxed put back. */
function rulesTableV10(): string {
  const v11 = rulesTableV11();
  const relaxed = `("rules"."reference" IS NOT NULL OR json_extract("rules"."match_json", '$.operator') IS NULL)`;
  expect(v11).toContain(relaxed);
  return v11.replace(relaxed, `"rules"."reference" IS NOT NULL`);
}

/** Digest of a document under any kind a schema has stored. */
const digestOf = (kind: string, rule: object): string => ruleDocumentDigest({ kind, rule } as unknown as RuleDocument);

/** A policy as v11 stored it (a severity on every finding, no mode) and the v15 body the upgrades turn it into. */
function legacyCodeRule(id: string, severities: readonly string[], mode: RuleMode, description = "Bundled policy") {
  const checks = severities.map((severity, index) => ({
    id: `check-${index}`,
    forbid: { regex: { source: "file.content", pattern: "legacy" } },
    finding: { anchor: "witness.span", code: "LEGACY", severity, message: "Legacy finding" },
  }));
  const legacy = { format: "rerail.policy/v1", id, revision: 1, description, checks };
  const current: CodeRule = codeRuleSchema.parse({
    format: "rerail.policy/v1",
    id,
    revision: 1,
    description,
    mode,
    clauses: checks.map(({ finding: { severity: _severity, ...violation }, ...clause }) => ({ ...clause, violation })),
  });
  return { legacy, current };
}

/** A cosine rule as v11 stored it (`action.delivery`, no mode) and the body the upgrades turn it into. */
function legacyCosine(id: string, delivery: string, mode: RuleMode, reference = "Legacy reference") {
  const legacy = {
    id,
    reference,
    match: { operator: "gte", threshold: 0.6 },
    sample: { source: "user-input", boundary: "message" },
    action: { delivery, content: "Legacy guidance" },
  };
  const current: ConversationRule = parseConversationRule({ ...legacy, mode, action: { content: legacy.action.content } });
  return { legacy, current };
}

function insertLegacyCodeRule(
  database: Database.Database,
  rule: Omit<ReturnType<typeof legacyCodeRule>["legacy"], "checks"> & { checks: readonly object[] },
  sourceDigest: string | null,
): void {
  database.prepare(
    "INSERT INTO rules (id, kind, revision, enabled, scope, created_at, updated_at, source_digest, format, description, checks_json) VALUES (?, 'policy', ?, 1, 'repository', 1, 1, ?, ?, ?, ?)",
  ).run(rule.id, rule.revision, sourceDigest, rule.format, rule.description, JSON.stringify(rule.checks));
}

function insertLegacyCosine(database: Database.Database, rule: ReturnType<typeof legacyCosine>["legacy"], sourceDigest: string | null): void {
  database.prepare(
    "INSERT INTO rules (id, kind, revision, enabled, scope, created_at, updated_at, source_digest, reference, match_json, sample_json, action_json) VALUES (?, 'cosine', 1, 1, 'repository', 1, 1, ?, ?, ?, ?, ?)",
  ).run(rule.id, sourceDigest, rule.reference, JSON.stringify(rule.match), JSON.stringify(rule.sample), JSON.stringify(rule.action));
}

/** Every row the rule-mode and v15 upgrades rewrite, to show a second pass changes nothing. */
function upgradedRows(path: string): unknown[] {
  const database = new Database(path, { readonly: true });
  try {
    return ["rules", "catalog_rules", "audits"].map(table => database.prepare(`SELECT * FROM ${table}`).all());
  } finally {
    database.close();
  }
}

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function temporaryDatabase(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-rules-"));
  temporaryDirectories.push(root);
  return join(root, ".rerail", "rerail.sqlite");
}

describe("Rerail schema", () => {
  it("keeps the committed DDL in sync with the Drizzle tables", async () => {
    expect(await readFile(SCHEMA_SQL_PATH, "utf8")).toBe(await renderRerailSchemaModule());
  });

  it("adds presentation settings to a v6 database without changing its durable records", async () => {
    const path = await temporaryDatabase();
    const legacy = createV14Database(path);
    legacy.exec(`
      INSERT INTO sessions (session_id, host, cwd, started_at, ended_at) VALUES ('${MOCK_SESSION_ID}', 'omp', '/workspace', 40000, 80000);
      INSERT INTO events (session_id, event_id, kind, observed_at, payload_json) VALUES ('${MOCK_SESSION_ID}', 'mock-event-1', 'user-prompt', 50000, '{"text":"hi"}');
      INSERT INTO work_links (session_id, kind, reference, url, created_at) VALUES ('${MOCK_SESSION_ID}', 'git-branch', 'feature/x', NULL, 100000);
      INSERT INTO policy_runs (run_id, session_id, workspace_root, scope, mode, revision, paths_json,
        started_at, status, findings_json, failures_json, rule_provenance_json, rule_revisions_json)
        VALUES ('running', '${MOCK_SESSION_ID}', '/workspace', 'repository', 'log', 1, '[]', 100000, 'running', '[]', '[]', '[]', '[]');
      DROP TABLE presentation_preferences;
      DROP TABLE rerail_schema_revision;
      PRAGMA user_version = 6;
    `);
    const tables = ["sessions", "events", "work_links"];
    const [sessions = [], ...records] = tables.map(table => legacy.prepare(`SELECT * FROM ${table}`).all() as Array<Record<string, unknown>>);
    const { run_id: auditId, findings_json: violationsJson, ...run } = legacy.prepare("SELECT * FROM policy_runs").get() as Record<string, unknown>;
    legacy.close();

    const preferences = new SqlitePresentationPreferenceStore(path);
    try {
      preferences.upsert({ repository: "", host: "", presence: "silent" });
      expect(preferences.effective("/workspace", "omp")).toBe("silent");
    } finally {
      preferences.close();
    }
    const upgraded = new Database(path, { readonly: true });
    try {
      // v16 adds `start_gated`; a session an older release recorded gets 0, so the start carry-over may count it.
      expect(tables.map(table => upgraded.prepare(`SELECT * FROM ${table}`).all()))
        .toEqual([sessions.map(session => ({ ...session, start_gated: 0 })), ...records]);
      expect(upgraded.prepare("SELECT * FROM audits").all()).toEqual([{ ...run, audit_id: auditId, violations_json: violationsJson }]);
    } finally {
      upgraded.close();
    }
  });
  it("moves a database created before this schema aside and starts fresh instead of migrating it", async () => {
    const path = await temporaryDatabase();
    const legacy = new SqliteSessionStore(path);
    legacy.close();
    const raw = new Database(path);
    raw.exec("CREATE TABLE transcript_sessions (session_id TEXT PRIMARY KEY); INSERT INTO transcript_sessions VALUES ('old')");
    raw.close();
    const warnings: string[] = [];
    const write = vi.spyOn(process.stderr, "write").mockImplementation(chunk => {
      warnings.push(String(chunk));
      return true;
    });

    try {
      openRerailDatabase(path).$client.close();
    } finally {
      write.mockRestore();
    }

    const archives = (await readdir(dirname(path))).filter(name => name.startsWith("rerail.sqlite.pre-schema-"));
    expect(archives).toHaveLength(1);
    const fresh = new Database(path, { readonly: true });
    expect(fresh.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'transcript_%'").all()).toEqual([]);
    expect(fresh.prepare("SELECT count(*) AS n FROM rules").get()).toEqual({ n: 0 });
    fresh.close();
    const archived = new Database(join(dirname(path), archives[0]!), { readonly: true });
    expect(archived.prepare("SELECT session_id FROM transcript_sessions").all()).toEqual([{ session_id: "old" }]);
    archived.close();
  });
});

describe("SqliteRuleStore", () => {
  it("round-trips code rules and conversation rules through kind-specific columns", async () => {
    const path = await temporaryDatabase();
    const store = new SqliteRuleStore(path);
    const codeRule = MOCK_CODE_RULES[0]!;
    const conversationRule = MOCK_CONVERSATION_RULES[0]!;

    const storedCodeRule = store.upsertCodeRule(codeRule, { now: 1_000 });
    const storedConversationRule = store.upsertConversationRule(conversationRule, { now: 1_000, scope: "global" });

    expect(storedCodeRule).toEqual({
      kind: "file",
      enabled: true,
      scope: "repository",
      revision: codeRule.revision,
      createdAt: 1_000,
      updatedAt: 1_000,
      sourceDigest: null,
      rule: codeRule,
    });
    expect(storedConversationRule).toEqual({
      kind: "conversation",
      enabled: true,
      scope: "global",
      revision: 1,
      createdAt: 1_000,
      updatedAt: 1_000,
      sourceDigest: null,
      rule: conversationRule,
    });
    store.close();

    const database = new Database(path, { readonly: true });
    expect(database.prepare(
      "SELECT id, kind, format, json_extract(scope_json, '$.include[0]') AS firstInclude, json_array_length(clauses_json) AS clauseCount, reference FROM rules ORDER BY id",
    ).all()).toEqual([
      { id: conversationRule.id, kind: "conversation", format: null, firstInclude: null, clauseCount: null, reference: conversationRule.reference },
      { id: codeRule.id, kind: "file", format: "rerail.policy/v1", firstInclude: codeRule.scope!.include[0], clauseCount: codeRule.clauses.length, reference: null },
    ]);
    expect(database.prepare("SELECT id, mode FROM rules ORDER BY id").all()).toEqual([
      { id: conversationRule.id, mode: conversationRule.mode },
      { id: codeRule.id, mode: codeRule.mode },
    ]);
    database.close();
  });

  it("rejects invalid bodies before they reach SQLite", async () => {
    const store = new SqliteRuleStore(await temporaryDatabase());
    const codeRule = MOCK_CODE_RULES[0]!;

    expect(() => store.upsertCodeRule({ ...codeRule, clauses: [] })).toThrow();
    expect(() => store.upsertCodeRule({ ...codeRule, format: "rerail.policy/v1" })).toThrow(/rerail\.policy\/v1/u);
    expect(() => store.upsertCodeRule({ ...codeRule, clauses: [{ ...codeRule.clauses[0]!, budget: { query: { evidence: { as: "x", query: { ast: { language: "python", rule: { kind: "call" } } }, project: { path: "file.path" } } }, count: { lte: 1 } } }] })).toThrow(/exactly one constraint form/u);
    expect(() => store.upsertCodeRule({ ...codeRule, clauses: [codeRule.clauses[0]!, { ...codeRule.clauses[0]! }] })).toThrow(/Duplicate clause id/u);
    expect(() => store.upsertCodeRule({ ...codeRule, clauses: [{ ...codeRule.clauses[0]!, forbid: { regex: { source: "stdin", pattern: "x" } } }] })).toThrow();
    expect(() => store.upsertConversationRule({ id: "bad", reference: "x", match: { operator: "gte", threshold: 2 }, sample: { source: "user-input", boundary: "message" }, mode: "steer", action: { content: "y" } })).toThrow(/threshold/u);
    expect(store.list()).toEqual([]);
    store.close();
  });

  it("stores a structural tool-call rule without a reference", async () => {
    const path = await temporaryDatabase();
    const store = new SqliteRuleStore(path);
    const rule = TOOL_CALL_RULES[0]!;

    expect(store.upsertConversationRule(rule, { now: 1_000 }).rule).toEqual(rule);
    expect(store.conversationRules()).toEqual([rule]);
    store.close();

    const database = new Database(path, { readonly: true });
    expect(database.prepare("SELECT reference, json_extract(match_json, '$.all[1].field') AS field FROM rules").get())
      .toEqual({ reference: null, field: "input.command" });
    database.close();
  });

  it("upgrades a v10 rules table so structural rules fit beside its existing cosine rows", async () => {
    const path = await temporaryDatabase();
    const conversationRule = MOCK_CONVERSATION_RULES[0]!;
    const legacy = createV14Database(path);
    // A v10 file predates the compatibility-floor stamp, so it has no revision table either.
    legacy.exec(`DROP TABLE rules; DROP TABLE rerail_schema_revision; ${rulesTableV10()}; PRAGMA user_version = 10;`);
    legacy.prepare(
      "INSERT INTO rules (id, kind, revision, enabled, scope, created_at, updated_at, reference, match_json, sample_json, action_json, repeat_json) VALUES (?, 'cosine', 3, 1, 'repository', 1, 2, ?, ?, ?, ?, ?)",
    ).run(conversationRule.id, conversationRule.reference, JSON.stringify(conversationRule.match), JSON.stringify(conversationRule.sample), JSON.stringify({ delivery: "steer", ...conversationRule.action }), conversationRule.repeat === undefined ? null : JSON.stringify(conversationRule.repeat));
    expect(() => legacy.prepare(
      "INSERT INTO rules (id, kind, revision, enabled, scope, created_at, updated_at, match_json, sample_json, action_json) VALUES ('x', 'cosine', 1, 1, 'repository', 1, 1, '{}', '{}', '{}')",
    ).run()).toThrow(/rules_cosine_columns/u);
    legacy.close();

    const store = new SqliteRuleStore(path);
    expect(store.get(conversationRule.id)).toMatchObject({ revision: 3, createdAt: 1, updatedAt: 2, rule: conversationRule });
    store.upsertConversationRule(TOOL_CALL_RULES[0]!, { now: 5 });
    expect(store.list({ kind: "conversation" }).map(stored => stored.rule.id)).toEqual([conversationRule.id, TOOL_CALL_RULES[0]!.id].sort());
    store.close();

    const upgraded = new Database(path, { readonly: true });
    expect(upgraded.pragma("user_version", { simple: true })).toBe(RERAIL_SCHEMA_COMPATIBILITY_FLOOR);
    expect(upgraded.prepare("SELECT revision FROM rerail_schema_revision").pluck().get()).toBe(RERAIL_SCHEMA_VERSION);
    expect(upgraded.prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'rules_kind_enabled'").all()).toHaveLength(1);
    expect(() => upgraded.prepare("SELECT 1 FROM rules_legacy").all()).toThrow(/no such table/u);
    upgraded.close();
  });

  it("folds v11 finding severities and cosine deliveries into one rule mode and reaches v15, once", async () => {
    const path = await temporaryDatabase();
    const legacy = createV14Database(path);
    legacy.exec(`DROP TABLE rules; ${rulesTableV11()}; PRAGMA user_version = 11;`);
    const bundled = legacyCodeRule("bundled", ["advisory", "warning", "error"], "steer");
    const edited = legacyCodeRule("edited", ["advisory"], "log");
    const created = legacyCodeRule("created", ["warning", "advisory"], "steer");
    const seededFrom = legacyCodeRule("edited", ["advisory"], "log", "Before the user's edit").legacy;
    insertLegacyCodeRule(legacy, bundled.legacy, digestOf("policy", bundled.legacy));
    insertLegacyCodeRule(legacy, edited.legacy, digestOf("policy", seededFrom));
    insertLegacyCodeRule(legacy, created.legacy, null);
    const cosines = [["observe", "log"], ["steer", "steer"], ["followUp", "steer"], ["nextTurn", "steer"]] as const;
    for (const [delivery, mode] of cosines) {
      const { legacy: rule } = legacyCosine(`cosine-${delivery}`, delivery, mode);
      insertLegacyCosine(legacy, rule, digestOf("cosine", rule));
    }
    const finding = { policyId: "bundled", checkId: "clause-1", code: "LEGACY", message: "Legacy finding", captures: {}, variables: {} };
    legacy.prepare(
      "INSERT INTO policy_runs (run_id, session_id, workspace_root, scope, mode, revision, paths_json, started_at, status, findings_json, failures_json, rule_provenance_json, rule_revisions_json) VALUES ('run', 'session', '/workspace', 'repository', 'log', 1, '[]', 1, 'completed', ?, '[]', '[]', '[]')",
    ).run(JSON.stringify([
      { id: "a", ...finding, severity: "warning", path: "a.py" },
      { id: "b", ...finding, severity: "advisory", path: "b.py" },
    ]));
    legacy.close();

    const store = new SqliteRuleStore(path);
    expect(Object.fromEntries(store.list().map(stored => [stored.rule.id, stored.rule.mode]))).toEqual({
      bundled: "steer",
      edited: "log",
      created: "steer",
      ...Object.fromEntries(cosines.map(([delivery, mode]) => [`cosine-${delivery}`, mode])),
    });
    expect(store.get("bundled")).toMatchObject({ kind: "file", rule: bundled.current, sourceDigest: digestOf("file", bundled.current) });
    expect(store.get("edited")).toMatchObject({ rule: edited.current, sourceDigest: digestOf("policy", seededFrom) });
    expect(store.get("created")).toMatchObject({ rule: created.current, sourceDigest: null });
    expect(store.get("cosine-followUp")).toMatchObject({
      kind: "conversation",
      rule: legacyCosine("cosine-followUp", "followUp", "steer").current,
      sourceDigest: digestOf("conversation", legacyCosine("cosine-followUp", "followUp", "steer").current),
    });
    // The unedited bundled row reads as seeded from the bundle, so it follows the next bundle; the edited one stays.
    expect(store.seed([{ kind: "file", rule: bundled.current }])).toEqual({ inserted: 0, refreshed: 0, kept: 1 });
    expect(store.seed([
      { kind: "file", rule: { ...bundled.current, description: "Updated bundle" } },
      { kind: "file", rule: { ...edited.current, description: "Updated bundle" } },
    ])).toEqual({ inserted: 0, refreshed: 1, kept: 1 });
    expect(store.get("bundled")?.rule).toMatchObject({ description: "Updated bundle" });
    expect(store.get("edited")?.rule).toMatchObject({ description: "Bundled policy" });
    store.close();

    const raw = new Database(path);
    const violations = JSON.parse(raw.prepare("SELECT violations_json FROM audits").pluck().get() as string) as Record<string, unknown>[];
    const violation = { ruleId: "bundled", clauseId: "clause-1", code: "LEGACY", message: "Legacy finding", captures: {}, variables: {} };
    expect(violations).toEqual([
      { id: "a", ...violation, mode: "steer", path: "a.py" },
      { id: "b", ...violation, mode: "log", path: "b.py" },
    ]);
    expect(Object.keys(violations[0]!)).toEqual(["id", ...Object.keys(violation), "mode", "path"]);
    raw.pragma("user_version = 11");
    raw.close();
    const before = upgradedRows(path);
    openRerailDatabase(path).$client.close();
    expect(upgradedRows(path)).toEqual(before);
  });

  it("rewrites the findings of a revision-12 database's result pages", async () => {
    const path = await temporaryDatabase();
    const legacy = createV14Database(path);
    const finding = { policyId: "p", checkId: "c", code: "LEGACY", message: "Legacy finding", captures: {}, variables: {} };
    const findings = JSON.stringify([{ id: "a", ...finding, severity: "error" }, { id: "b", ...finding, severity: "advisory" }]);
    legacy.prepare(
      "INSERT INTO policy_runs (run_id, session_id, workspace_root, scope, mode, revision, paths_json, started_at, status, findings_json, failures_json, rule_provenance_json, rule_revisions_json) VALUES ('run', 'session', '/workspace', 'repository', 'log', 1, '[]', 1, 'completed', ?, '[]', '[]', '[]')",
    ).run(findings);
    legacy.prepare("INSERT INTO policy_run_result_pages (run_id, page_index, findings_json, failures_json) VALUES ('run', 0, ?, '[]')").run(findings);
    legacy.exec("UPDATE rerail_schema_revision SET revision = 12; PRAGMA user_version = 10;");
    legacy.close();

    openRerailDatabase(path).$client.close();
    const upgraded = new Database(path, { readonly: true });
    const modes = (sql: string) => (JSON.parse(upgraded.prepare(sql).pluck().get() as string) as Record<string, unknown>[])
      .map(({ id, mode, severity }) => ({ id, mode, severity }));
    const expected = [{ id: "a", mode: "steer", severity: undefined }, { id: "b", mode: "log", severity: undefined }];
    expect(modes("SELECT violations_json FROM audit_result_pages")).toEqual(expected);
    expect(modes("SELECT violations_json FROM audits")).toEqual(expected);
    expect(upgraded.pragma("user_version", { simple: true })).toBe(RERAIL_SCHEMA_COMPATIBILITY_FLOOR);
    upgraded.close();
  });

  it("rewrites v11 catalog documents and re-stamps their digests", async () => {
    const path = await temporaryDatabase();
    const repository = { repositoryId: "repo", workspaceRoot: "/repo" };
    const bundled = legacyCodeRule("bundled", ["warning"], "steer");
    const edited = legacyCosine("edited", "nextTurn", "steer");
    const seededFrom = legacyCosine("edited", "nextTurn", "steer", "Before the user's edit").legacy;
    const legacy = createV14Database(path);
    legacy.exec(
      "INSERT INTO rule_projects (project_id, name, created_at, updated_at) VALUES ('repository:repo', 'repo', 1, 1);"
      + "INSERT INTO rule_repositories (repository_id, last_known_path, created_at, updated_at) VALUES ('repo', '/repo', 1, 1);"
      + "INSERT INTO rule_project_repositories (project_id, repository_id, added_at) VALUES ('repository:repo', 'repo', 1);",
    );
    const insert = legacy.prepare(
      "INSERT INTO catalog_rules (owner_kind, owner_id, rule_id, kind, revision, enabled, document_json, payload_hash, source_digest, origin, created_at, updated_at) "
      + "VALUES ('project', 'repository:repo', ?, ?, 1, 1, ?, ?, ?, 'bundled', 1000, 1000)",
    );
    insert.run("bundled", "policy", JSON.stringify({ kind: "policy", rule: bundled.legacy }), digestOf("policy", bundled.legacy), digestOf("policy", bundled.legacy));
    insert.run("edited", "cosine", JSON.stringify({ kind: "cosine", rule: edited.legacy }), digestOf("cosine", edited.legacy), digestOf("cosine", seededFrom));
    legacy.exec(
      "INSERT INTO rule_catalog_identities (rule_id, kind) VALUES ('bundled', 'policy'), ('edited', 'cosine');"
      + `DROP TABLE rules; ${rulesTableV11()}; PRAGMA user_version = 11;`,
    );
    legacy.close();

    const upgraded = ScopedRuleCatalog.open(path, repository);
    expect(upgraded.get("bundled")).toMatchObject({
      kind: "file",
      rule: bundled.current,
      payloadHash: digestOf("file", bundled.current),
      sourceDigest: digestOf("file", bundled.current),
    });
    expect(upgraded.get("edited")).toMatchObject({
      kind: "conversation",
      rule: edited.current,
      payloadHash: digestOf("conversation", edited.current),
      sourceDigest: digestOf("cosine", seededFrom),
    });
    expect(upgraded.seed([
      { kind: "file", rule: { ...bundled.current, description: "Updated bundle" } },
      { kind: "conversation", rule: { ...edited.current, reference: "Updated bundle" } },
    ], 2_000)).toEqual({ inserted: 0, refreshed: 1, kept: 1 });
    expect(upgraded.get("bundled")?.rule).toMatchObject({ description: "Updated bundle" });
    expect(upgraded.get("edited")?.rule).toMatchObject({ reference: "Legacy reference" });
    upgraded.close();
  });

  it("leaves a define check unchanged and takes the mode from the checks that report", async () => {
    const path = await temporaryDatabase();
    const repository = { repositoryId: "repo", workspaceRoot: "/repo" };
    const define = { id: "options", define: { options: { strict: { type: "boolean", default: true } } } };
    const withDefine = (id: string) => {
      const { legacy, current } = legacyCodeRule(id, ["advisory", "warning"], "steer");
      return {
        legacy: { ...legacy, checks: [define, ...legacy.checks] },
        current: codeRuleSchema.parse({ ...current, clauses: [define, ...current.clauses] }),
      };
    };
    const stored = withDefine("stored");
    const cataloged = withDefine("cataloged");
    const legacy = createV14Database(path);
    legacy.exec(
      "INSERT INTO rule_projects (project_id, name, created_at, updated_at) VALUES ('repository:repo', 'repo', 1, 1);"
      + "INSERT INTO rule_repositories (repository_id, last_known_path, created_at, updated_at) VALUES ('repo', '/repo', 1, 1);"
      + "INSERT INTO rule_project_repositories (project_id, repository_id, added_at) VALUES ('repository:repo', 'repo', 1);"
      + "INSERT INTO rule_catalog_identities (rule_id, kind) VALUES ('cataloged', 'policy');",
    );
    legacy.prepare(
      "INSERT INTO catalog_rules (owner_kind, owner_id, rule_id, kind, revision, enabled, document_json, payload_hash, source_digest, origin, created_at, updated_at) "
      + "VALUES ('project', 'repository:repo', 'cataloged', 'policy', 1, 1, ?, ?, ?, 'bundled', 1000, 1000)",
    ).run(JSON.stringify({ kind: "policy", rule: cataloged.legacy }), digestOf("policy", cataloged.legacy), digestOf("policy", cataloged.legacy));
    legacy.exec(`DROP TABLE rules; ${rulesTableV11()}; PRAGMA user_version = 11;`);
    insertLegacyCodeRule(legacy, stored.legacy, digestOf("policy", stored.legacy));
    legacy.close();

    const store = new SqliteRuleStore(path);
    expect(store.get("stored")).toMatchObject({ kind: "file", rule: stored.current, sourceDigest: digestOf("file", stored.current) });
    expect((store.get("stored")?.rule as CodeRule).clauses[0]).toEqual(define);
    store.close();
    const upgraded = ScopedRuleCatalog.open(path, repository);
    expect(upgraded.get("cataloged")).toMatchObject({ kind: "file", rule: cataloged.current, sourceDigest: digestOf("file", cataloged.current) });
    upgraded.close();
  });

  it("refuses to guess a mode for a value v11 never accepted", async () => {
    const path = await temporaryDatabase();
    const legacy = createV14Database(path);
    legacy.exec(`DROP TABLE rules; ${rulesTableV11()}; PRAGMA user_version = 11;`);
    insertLegacyCosine(legacy, legacyCosine("shouting", "steer", "steer").legacy, null);
    legacy.prepare("UPDATE rules SET action_json = json_set(action_json, '$.delivery', 'shout')").run();
    legacy.close();

    expect(() => openRerailDatabase(path)).toThrow("Cannot upgrade rule shouting to rule modes: action delivery \"shout\" has no rule mode");
    const unchanged = new Database(path, { readonly: true });
    expect(unchanged.pragma("user_version", { simple: true })).toBe(11);
    expect(unchanged.prepare("SELECT json_extract(action_json, '$.delivery') AS delivery FROM rules").get()).toEqual({ delivery: "shout" });
    unchanged.close();
  });

  it("rebuilds the rules table of a floor-stamped revision-11 file that already has result pages", async () => {
    const path = await temporaryDatabase();
    const stamped = createV14Database(path);
    stamped.exec(`DROP TABLE rules; ${rulesTableV10()}; UPDATE rerail_schema_revision SET revision = 11; PRAGMA user_version = 10;`);
    stamped.close();

    const store = new SqliteRuleStore(path);
    store.upsertConversationRule(TOOL_CALL_RULES[0]!, { now: 5 });
    expect(store.list({ kind: "conversation" }).map(stored => stored.rule.id)).toEqual([TOOL_CALL_RULES[0]!.id]);
    store.close();
  });

  it("still requires a reference for a similarity rule row", async () => {
    const path = await temporaryDatabase();
    new SqliteRuleStore(path).close();
    const database = new Database(path);
    expect(() => database.prepare(
      "INSERT INTO rules (id, kind, mode, revision, enabled, scope, created_at, updated_at, match_json, sample_json, action_json) VALUES ('x', 'conversation', 'steer', 1, 1, 'repository', 1, 1, '{\"operator\":\"gte\",\"threshold\":0.5}', '{}', '{}')",
    ).run()).toThrow(/rules_conversation_columns/u);
    database.close();
  });

  it("stores only the three rule modes", async () => {
    const path = await temporaryDatabase();
    new SqliteRuleStore(path).close();
    const database = new Database(path);
    expect(() => database.prepare(
      "INSERT INTO rules (id, kind, mode, revision, enabled, scope, created_at, updated_at, format, clauses_json) VALUES ('x', 'file', 'error', 1, 1, 'repository', 1, 1, 'rerail.policy/v1', '[]')",
    ).run()).toThrow(/rules_mode/u);
    database.close();
  });

  it("keeps ids unique across kinds", async () => {
    const store = new SqliteRuleStore(await temporaryDatabase());
    store.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id: "shared" });

    expect(() => store.upsertCodeRule({ ...MOCK_CODE_RULES[0]!, id: "shared" })).toThrow(/exists with kind conversation/u);
    store.close();
  });

  it("updates in place, bumps conversation rule revisions, and preserves created_at", async () => {
    const store = new SqliteRuleStore(await temporaryDatabase());
    const conversationRule = MOCK_CONVERSATION_RULES[0]!;
    store.upsertConversationRule(conversationRule, { now: 1_000, enabled: false });

    const updated = store.upsertConversationRule({ ...conversationRule, match: { operator: "gte", threshold: 0.8 } }, { now: 2_000 });

    expect(updated.revision).toBe(2);
    expect(updated.enabled).toBe(false);
    expect(updated.createdAt).toBe(1_000);
    expect(updated.updatedAt).toBe(2_000);
    expect(updated.rule.match).toEqual({ operator: "gte", threshold: 0.8 });
    store.close();
  });

  it("filters by kind, enabled, and scope without parsing JSON, and bulk toggles", async () => {
    const store = new SqliteRuleStore(await temporaryDatabase());
    for (const rule of MOCK_CODE_RULES) store.upsertCodeRule(rule, { now: 1 });
    store.upsertConversationRule(MOCK_CONVERSATION_RULES[0]!, { now: 1 });
    store.upsertConversationRule(MOCK_CONVERSATION_RULES[1]!, { now: 1, enabled: false, scope: "global" });

    expect(store.list({ kind: "file" }).map(stored => stored.rule.id)).toEqual(MOCK_CODE_RULES.map(rule => rule.id).sort());
    expect(store.list({ enabled: false }).map(stored => stored.rule.id)).toEqual([MOCK_CONVERSATION_RULES[1]!.id]);
    expect(store.list({ scope: "global" }).map(stored => stored.rule.id)).toEqual([MOCK_CONVERSATION_RULES[1]!.id]);
    expect(store.conversationRules()).toEqual([MOCK_CONVERSATION_RULES[0]]);
    expect(store.codeRules()).toHaveLength(3);

    expect(store.setEnabled({ ids: [MOCK_CONVERSATION_RULES[0]!.id, MOCK_CONVERSATION_RULES[1]!.id, "missing"] }, false, 5)).toBe(1);
    expect(store.conversationRules()).toEqual([]);
    expect(store.get(MOCK_CONVERSATION_RULES[0]!.id)?.updatedAt).toBe(5);
    expect(store.remove([MOCK_CONVERSATION_RULES[0]!.id, MOCK_CONVERSATION_RULES[0]!.id])).toBe(1);
    expect(store.get(MOCK_CONVERSATION_RULES[0]!.id)).toBeUndefined();
    store.close();
  });

  it("searches text and conversation rule sample fields, and toggles a filtered set", async () => {
    const store = new SqliteRuleStore(await temporaryDatabase());
    for (const rule of MOCK_CODE_RULES) store.upsertCodeRule(rule, { now: 1 });
    for (const rule of MOCK_CONVERSATION_RULES) store.upsertConversationRule(rule, { now: 1 });
    const ids = (query: Parameters<typeof store.list>[0]) => store.list(query).map(stored => stored.rule.id);

    expect(ids({ text: "GENERATED client" })).toEqual(["mock-cosine-prefer-generated-client", "mock-generated-client-imports"]);
    expect(ids({ text: "migration" })).toEqual(["mock-cosine-migration-warning"]);
    expect(ids({ text: "lazy query" })).toEqual(["mock-lazy-query-shaping"]);
    expect(ids({ text: "100%" })).toEqual([]);
    expect(ids({ source: "user-input" })).toEqual(["mock-cosine-migration-warning"]);
    expect(ids({ boundary: "sentence", mode: "steer" })).toEqual(["mock-cosine-prefer-generated-client"]);
    expect(ids({ kind: "file", mode: "steer" })).toEqual(["mock-lazy-query-shaping", "mock-no-console-log"]);
    expect(ids({ mode: "log" })).toEqual(["mock-generated-client-imports"]);

    expect(store.setEnabled({ filter: { text: "generated client" } }, false, 7)).toBe(2);
    expect(ids({ enabled: false })).toEqual(["mock-cosine-prefer-generated-client", "mock-generated-client-imports"]);
    expect(store.setEnabled({ filter: { kind: "conversation" } }, false)).toBe(1);
    expect(store.setEnabled({ filter: {} }, true)).toBe(3);
    expect(store.setEnabled({ ids: [] }, false)).toBe(0);
    store.close();
  });
});

describe("work links", () => {
  it("associates a session with external references without enforcing the session exists", async () => {
    const path = await temporaryDatabase();
    const store = new SqliteSessionStore(path);
    store.linkWork({ sessionId: "unknown-session", kind: "git-branch", reference: "feature/x" }, 10);
    store.linkWork({ sessionId: "unknown-session", kind: "linear-issue", reference: "NER-1", url: "https://linear.app/nera/issue/NER-1" }, 11);
    store.linkWork({ sessionId: "unknown-session", kind: "git-branch", reference: "feature/x", url: "https://example.test/tree/feature/x" }, 12);

    const expected = [
      { linkId: 1, sessionId: "unknown-session", kind: "git-branch", reference: "feature/x", url: "https://example.test/tree/feature/x", createdAt: 10 },
      { linkId: 2, sessionId: "unknown-session", kind: "linear-issue", reference: "NER-1", url: "https://linear.app/nera/issue/NER-1", createdAt: 11 },
    ];
    expect(store.workLinks("unknown-session")).toEqual(expected);
    store.close();

    const reader = new SqliteSessionReader(path);
    expect(reader.workLinks("unknown-session")).toEqual(expected);
    expect(reader.sessions()).toEqual([]);
    reader.close();
  });
});

describe("seedRerailDatabase", () => {
  it("populates mock rules, a session, and work links idempotently", async () => {
    const path = await temporaryDatabase();

    const first = seedRerailDatabase(path, { cwd: "/workspace", now: 100_000 });
    const second = seedRerailDatabase(path, { cwd: "/workspace", now: 200_000 });

    expect(first).toEqual({ codeRules: 3, conversationRules: 2, sessions: 1, events: 4, workLinks: 2 });
    expect(second).toEqual(first);
    const rules = new SqliteRuleStore(path);
    expect(rules.list().map(stored => [stored.rule.id, stored.kind, stored.enabled])).toEqual([
      ["mock-cosine-migration-warning", "conversation", false],
      ["mock-cosine-prefer-generated-client", "conversation", true],
      ["mock-generated-client-imports", "file", true],
      ["mock-lazy-query-shaping", "file", true],
      ["mock-no-console-log", "file", true],
    ]);
    expect(rules.get("mock-no-console-log")?.createdAt).toBe(100_000);
    rules.close();
    const reader = new SqliteSessionReader(path);
    expect(reader.sessions()).toEqual([expect.objectContaining({ sessionId: MOCK_SESSION_ID, host: "omp", cwd: "/workspace", eventCount: 4 })]);
    expect(reader.workLinks(MOCK_SESSION_ID).map(link => link.kind)).toEqual(["git-branch", "linear-issue"]);
    reader.close();
  });

  it("keeps each repository's sample session in that repository when two repositories seed one database", async () => {
    const path = await temporaryDatabase();

    seedRerailDatabase(path, { cwd: "/a", repositoryId: "repository-a", now: 100_000 });
    seedRerailDatabase(path, { cwd: "/b", repositoryId: "repository-b", now: 200_000 });

    for (const [repositoryId, cwd] of [["repository-a", "/a"], ["repository-b", "/b"]] as const) {
      const reader = new SqliteSessionReader(path, { repositoryId });
      const sessions = reader.sessions();
      expect(sessions).toEqual([expect.objectContaining({ sessionId: mockSessionId(repositoryId), repositoryId, cwd, eventCount: 4 })]);
      expect(reader.workLinks(mockSessionId(repositoryId)).map(link => link.kind)).toEqual(["git-branch", "linear-issue"]);
      reader.close();
    }
    expect(mockSessionId("repository-a")).not.toBe(mockSessionId("repository-b"));
  });
});
