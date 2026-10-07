import { readFileSync } from "node:fs";
import { readFile, mkdtemp, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { renderRerailSchemaModule, SCHEMA_SQL_PATH } from "../scripts/generate-schema-sql.js";
import { openRerailDatabase } from "../src/database.js";
import { RERAIL_SCHEMA_STATEMENTS } from "../src/schema-sql.js";
import { parseCosineRuleDocument } from "../src/rule-schema.js";
import { RERAIL_SCHEMA_VERSION } from "../src/schema.js";
import { SqliteRuleStore } from "../src/rule-store.js";
import { SqlitePresentationPreferenceStore } from "../src/presentation-preference-store.js";
import { MOCK_COSINE_RULES, MOCK_POLICY_RULES, MOCK_SESSION_ID, mockSessionId, seedRerailDatabase } from "../src/seed.js";
import { SqliteTranscriptReader, SqliteTranscriptStore } from "../src/transcript-store.js";

const TOOL_CALL_RULES = parseCosineRuleDocument(JSON.parse(readFileSync(resolve("test/fixtures/tool-call-rules.json"), "utf8")));

/** The v10 `rules` table: today's DDL with the one CHECK clause v11 relaxed put back. */
function rulesTableV10(): string {
  const current = RERAIL_SCHEMA_STATEMENTS.find(statement => statement.startsWith("CREATE TABLE IF NOT EXISTS `rules`"))!;
  const relaxed = `("rules"."reference" IS NOT NULL OR json_extract("rules"."match_json", '$.operator') IS NULL)`;
  expect(current).toContain(relaxed);
  return current.replace(relaxed, `"rules"."reference" IS NOT NULL`);
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
    seedRerailDatabase(path, { cwd: "/workspace", now: 100_000 });
    const legacy = new Database(path);
    legacy.exec(`
      INSERT INTO policy_runs (run_id, session_id, workspace_root, scope, mode, revision, paths_json,
        started_at, status, findings_json, failures_json, rule_provenance_json, rule_revisions_json)
        VALUES ('running', '${MOCK_SESSION_ID}', '/workspace', 'repository', 'log', 1, '[]', 100000, 'running', '[]', '[]', '[]', '[]');
      DROP TABLE presentation_preferences;
      PRAGMA user_version = 6;
    `);
    const tables = ["sessions", "events", "rules", "work_links", "policy_runs"];
    const records = tables.map(table => legacy.prepare(`SELECT * FROM ${table}`).all());
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
      expect(tables.map(table => upgraded.prepare(`SELECT * FROM ${table}`).all())).toEqual(records);
    } finally {
      upgraded.close();
    }
  });
  it("moves a database created before this schema aside and starts fresh instead of migrating it", async () => {
    const path = await temporaryDatabase();
    const legacy = new SqliteTranscriptStore(path);
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
  it("round-trips policy and cosine rules through kind-specific columns", async () => {
    const path = await temporaryDatabase();
    const store = new SqliteRuleStore(path);
    const policy = MOCK_POLICY_RULES[0]!;
    const cosine = MOCK_COSINE_RULES[0]!;

    const storedPolicy = store.upsertPolicy(policy, { now: 1_000 });
    const storedCosine = store.upsertCosine(cosine, { now: 1_000, scope: "global" });

    expect(storedPolicy).toEqual({
      kind: "policy",
      enabled: true,
      scope: "repository",
      revision: policy.revision,
      createdAt: 1_000,
      updatedAt: 1_000,
      sourceDigest: null,
      rule: policy,
    });
    expect(storedCosine).toEqual({
      kind: "cosine",
      enabled: true,
      scope: "global",
      revision: 1,
      createdAt: 1_000,
      updatedAt: 1_000,
      sourceDigest: null,
      rule: cosine,
    });
    store.close();

    const database = new Database(path, { readonly: true });
    expect(database.prepare(
      "SELECT id, kind, format, json_extract(scope_json, '$.include[0]') AS firstInclude, json_array_length(checks_json) AS checkCount, reference FROM rules ORDER BY id",
    ).all()).toEqual([
      { id: cosine.id, kind: "cosine", format: null, firstInclude: null, checkCount: null, reference: cosine.reference },
      { id: policy.id, kind: "policy", format: "rerail.policy/v1", firstInclude: policy.scope!.include[0], checkCount: policy.checks.length, reference: null },
    ]);
    expect(database.prepare(
      "SELECT id FROM rules WHERE EXISTS (SELECT 1 FROM json_each(checks_json) WHERE json_extract(value, '$.finding.severity') = 'error')",
    ).all()).toEqual([{ id: policy.id }]);
    database.close();
  });

  it("rejects invalid bodies before they reach SQLite", async () => {
    const store = new SqliteRuleStore(await temporaryDatabase());
    const policy = MOCK_POLICY_RULES[0]!;

    expect(() => store.upsertPolicy({ ...policy, checks: [] })).toThrow();
    expect(() => store.upsertPolicy({ ...policy, format: "rerail.policy/v2" })).toThrow(/rerail\.policy\/v1/u);
    expect(() => store.upsertPolicy({ ...policy, checks: [{ ...policy.checks[0]!, budget: { query: { evidence: { as: "x", query: { ast: { language: "python", rule: { kind: "call" } } }, project: { path: "file.path" } } }, count: { lte: 1 } } }] })).toThrow(/exactly one constraint form/u);
    expect(() => store.upsertPolicy({ ...policy, checks: [policy.checks[0]!, { ...policy.checks[0]! }] })).toThrow(/Duplicate check id/u);
    expect(() => store.upsertPolicy({ ...policy, checks: [{ ...policy.checks[0]!, forbid: { regex: { source: "stdin", pattern: "x" } } }] })).toThrow();
    expect(() => store.upsertCosine({ id: "bad", reference: "x", match: { operator: "gte", threshold: 2 }, sample: { source: "user-input", boundary: "message" }, action: { delivery: "steer", content: "y" } })).toThrow(/threshold/u);
    expect(store.list()).toEqual([]);
    store.close();
  });

  it("stores a structural tool-call rule without a reference", async () => {
    const path = await temporaryDatabase();
    const store = new SqliteRuleStore(path);
    const rule = TOOL_CALL_RULES[0]!;

    expect(store.upsertCosine(rule, { now: 1_000 }).rule).toEqual(rule);
    expect(store.cosineRules()).toEqual([rule]);
    store.close();

    const database = new Database(path, { readonly: true });
    expect(database.prepare("SELECT reference, json_extract(match_json, '$.all[1].field') AS field FROM rules").get())
      .toEqual({ reference: null, field: "input.command" });
    database.close();
  });

  it("upgrades a v10 rules table so structural rules fit beside its existing cosine rows", async () => {
    const path = await temporaryDatabase();
    const cosine = MOCK_COSINE_RULES[0]!;
    new SqliteRuleStore(path).close();
    const legacy = new Database(path);
    // A v10 file predates the compatibility-floor stamp, so it has no revision table either.
    legacy.exec(`DROP TABLE rules; DROP TABLE rerail_schema_revision; ${rulesTableV10()}; PRAGMA user_version = 10;`);
    legacy.prepare(
      "INSERT INTO rules (id, kind, revision, enabled, scope, created_at, updated_at, reference, match_json, sample_json, action_json, repeat_json) VALUES (?, 'cosine', 3, 1, 'repository', 1, 2, ?, ?, ?, ?, ?)",
    ).run(cosine.id, cosine.reference, JSON.stringify(cosine.match), JSON.stringify(cosine.sample), JSON.stringify(cosine.action), cosine.repeat === undefined ? null : JSON.stringify(cosine.repeat));
    expect(() => legacy.prepare(
      "INSERT INTO rules (id, kind, revision, enabled, scope, created_at, updated_at, match_json, sample_json, action_json) VALUES ('x', 'cosine', 1, 1, 'repository', 1, 1, '{}', '{}', '{}')",
    ).run()).toThrow(/rules_cosine_columns/u);
    legacy.close();

    const store = new SqliteRuleStore(path);
    expect(store.get(cosine.id)).toMatchObject({ revision: 3, createdAt: 1, updatedAt: 2, rule: cosine });
    store.upsertCosine(TOOL_CALL_RULES[0]!, { now: 5 });
    expect(store.list({ kind: "cosine" }).map(stored => stored.rule.id)).toEqual([cosine.id, TOOL_CALL_RULES[0]!.id].sort());
    store.close();

    const upgraded = new Database(path, { readonly: true });
    expect(upgraded.pragma("user_version", { simple: true })).toBe(10);
    expect(upgraded.prepare("SELECT revision FROM rerail_schema_revision").pluck().get()).toBe(RERAIL_SCHEMA_VERSION);
    expect(upgraded.prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'rules_kind_enabled'").all()).toHaveLength(1);
    expect(() => upgraded.prepare("SELECT 1 FROM rules_legacy_v11").all()).toThrow(/no such table/u);
    upgraded.close();
  });

  it("rebuilds the rules table of a floor-stamped revision-11 file that already has result pages", async () => {
    const path = await temporaryDatabase();
    new SqliteRuleStore(path).close();
    const stamped = new Database(path);
    stamped.exec(`DROP TABLE rules; ${rulesTableV10()}; UPDATE rerail_schema_revision SET revision = 11; PRAGMA user_version = 10;`);
    stamped.close();

    const store = new SqliteRuleStore(path);
    store.upsertCosine(TOOL_CALL_RULES[0]!, { now: 5 });
    expect(store.list({ kind: "cosine" }).map(stored => stored.rule.id)).toEqual([TOOL_CALL_RULES[0]!.id]);
    store.close();
  });

  it("still requires a reference for a similarity rule row", async () => {
    const path = await temporaryDatabase();
    new SqliteRuleStore(path).close();
    const database = new Database(path);
    expect(() => database.prepare(
      "INSERT INTO rules (id, kind, revision, enabled, scope, created_at, updated_at, match_json, sample_json, action_json) VALUES ('x', 'cosine', 1, 1, 'repository', 1, 1, '{\"operator\":\"gte\",\"threshold\":0.5}', '{}', '{}')",
    ).run()).toThrow(/rules_cosine_columns/u);
    database.close();
  });

  it("keeps ids unique across kinds", async () => {
    const store = new SqliteRuleStore(await temporaryDatabase());
    store.upsertCosine({ ...MOCK_COSINE_RULES[0]!, id: "shared" });

    expect(() => store.upsertPolicy({ ...MOCK_POLICY_RULES[0]!, id: "shared" })).toThrow(/exists with kind cosine/u);
    store.close();
  });

  it("updates in place, bumps cosine revisions, and preserves created_at", async () => {
    const store = new SqliteRuleStore(await temporaryDatabase());
    const cosine = MOCK_COSINE_RULES[0]!;
    store.upsertCosine(cosine, { now: 1_000, enabled: false });

    const updated = store.upsertCosine({ ...cosine, match: { operator: "gte", threshold: 0.8 } }, { now: 2_000 });

    expect(updated.revision).toBe(2);
    expect(updated.enabled).toBe(false);
    expect(updated.createdAt).toBe(1_000);
    expect(updated.updatedAt).toBe(2_000);
    expect(updated.rule.match).toEqual({ operator: "gte", threshold: 0.8 });
    store.close();
  });

  it("filters by kind, enabled, and scope without parsing JSON, and bulk toggles", async () => {
    const store = new SqliteRuleStore(await temporaryDatabase());
    for (const rule of MOCK_POLICY_RULES) store.upsertPolicy(rule, { now: 1 });
    store.upsertCosine(MOCK_COSINE_RULES[0]!, { now: 1 });
    store.upsertCosine(MOCK_COSINE_RULES[1]!, { now: 1, enabled: false, scope: "global" });

    expect(store.list({ kind: "policy" }).map(stored => stored.rule.id)).toEqual(MOCK_POLICY_RULES.map(rule => rule.id).sort());
    expect(store.list({ enabled: false }).map(stored => stored.rule.id)).toEqual([MOCK_COSINE_RULES[1]!.id]);
    expect(store.list({ scope: "global" }).map(stored => stored.rule.id)).toEqual([MOCK_COSINE_RULES[1]!.id]);
    expect(store.cosineRules()).toEqual([MOCK_COSINE_RULES[0]]);
    expect(store.policyRules()).toHaveLength(3);

    expect(store.setEnabled({ ids: [MOCK_COSINE_RULES[0]!.id, MOCK_COSINE_RULES[1]!.id, "missing"] }, false, 5)).toBe(1);
    expect(store.cosineRules()).toEqual([]);
    expect(store.get(MOCK_COSINE_RULES[0]!.id)?.updatedAt).toBe(5);
    expect(store.remove([MOCK_COSINE_RULES[0]!.id, MOCK_COSINE_RULES[0]!.id])).toBe(1);
    expect(store.get(MOCK_COSINE_RULES[0]!.id)).toBeUndefined();
    store.close();
  });

  it("searches text and cosine sample fields, and toggles a filtered set", async () => {
    const store = new SqliteRuleStore(await temporaryDatabase());
    for (const rule of MOCK_POLICY_RULES) store.upsertPolicy(rule, { now: 1 });
    for (const rule of MOCK_COSINE_RULES) store.upsertCosine(rule, { now: 1 });
    const ids = (query: Parameters<typeof store.list>[0]) => store.list(query).map(stored => stored.rule.id);

    expect(ids({ text: "GENERATED client" })).toEqual(["mock-cosine-prefer-generated-client", "mock-generated-client-imports"]);
    expect(ids({ text: "migration" })).toEqual(["mock-cosine-migration-warning"]);
    expect(ids({ text: "lazy query" })).toEqual(["mock-lazy-query-shaping"]);
    expect(ids({ text: "100%" })).toEqual([]);
    expect(ids({ source: "user-input" })).toEqual(["mock-cosine-migration-warning"]);
    expect(ids({ boundary: "sentence", delivery: "steer" })).toEqual(["mock-cosine-prefer-generated-client"]);
    expect(ids({ kind: "policy", delivery: "steer" })).toEqual([]);

    expect(store.setEnabled({ filter: { text: "generated client" } }, false, 7)).toBe(2);
    expect(ids({ enabled: false })).toEqual(["mock-cosine-prefer-generated-client", "mock-generated-client-imports"]);
    expect(store.setEnabled({ filter: { kind: "cosine" } }, false)).toBe(1);
    expect(store.setEnabled({ filter: {} }, true)).toBe(3);
    expect(store.setEnabled({ ids: [] }, false)).toBe(0);
    store.close();
  });
});

describe("work links", () => {
  it("associates a session with external references without enforcing the session exists", async () => {
    const path = await temporaryDatabase();
    const store = new SqliteTranscriptStore(path);
    store.linkWork({ sessionId: "unknown-session", kind: "git-branch", reference: "feature/x" }, 10);
    store.linkWork({ sessionId: "unknown-session", kind: "linear-issue", reference: "NER-1", url: "https://linear.app/nera/issue/NER-1" }, 11);
    store.linkWork({ sessionId: "unknown-session", kind: "git-branch", reference: "feature/x", url: "https://example.test/tree/feature/x" }, 12);

    const expected = [
      { linkId: 1, sessionId: "unknown-session", kind: "git-branch", reference: "feature/x", url: "https://example.test/tree/feature/x", createdAt: 10 },
      { linkId: 2, sessionId: "unknown-session", kind: "linear-issue", reference: "NER-1", url: "https://linear.app/nera/issue/NER-1", createdAt: 11 },
    ];
    expect(store.workLinks("unknown-session")).toEqual(expected);
    store.close();

    const reader = new SqliteTranscriptReader(path);
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

    expect(first).toEqual({ policyRules: 3, cosineRules: 2, sessions: 1, events: 4, workLinks: 2 });
    expect(second).toEqual(first);
    const rules = new SqliteRuleStore(path);
    expect(rules.list().map(stored => [stored.rule.id, stored.kind, stored.enabled])).toEqual([
      ["mock-cosine-migration-warning", "cosine", false],
      ["mock-cosine-prefer-generated-client", "cosine", true],
      ["mock-generated-client-imports", "policy", true],
      ["mock-lazy-query-shaping", "policy", true],
      ["mock-no-console-log", "policy", true],
    ]);
    expect(rules.get("mock-no-console-log")?.createdAt).toBe(100_000);
    rules.close();
    const reader = new SqliteTranscriptReader(path);
    expect(reader.sessions()).toEqual([expect.objectContaining({ sessionId: MOCK_SESSION_ID, host: "omp", cwd: "/workspace", eventCount: 4 })]);
    expect(reader.workLinks(MOCK_SESSION_ID).map(link => link.kind)).toEqual(["git-branch", "linear-issue"]);
    reader.close();
  });

  it("keeps each repository's sample session in that repository when two repositories seed one database", async () => {
    const path = await temporaryDatabase();

    seedRerailDatabase(path, { cwd: "/a", repositoryId: "repository-a", now: 100_000 });
    seedRerailDatabase(path, { cwd: "/b", repositoryId: "repository-b", now: 200_000 });

    for (const [repositoryId, cwd] of [["repository-a", "/a"], ["repository-b", "/b"]] as const) {
      const reader = new SqliteTranscriptReader(path, { repositoryId });
      const sessions = reader.sessions();
      expect(sessions).toEqual([expect.objectContaining({ sessionId: mockSessionId(repositoryId), repositoryId, cwd, eventCount: 4 })]);
      expect(reader.workLinks(mockSessionId(repositoryId)).map(link => link.kind)).toEqual(["git-branch", "linear-issue"]);
      reader.close();
    }
    expect(mockSessionId("repository-a")).not.toBe(mockSessionId("repository-b"));
  });
});
