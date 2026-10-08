import { readFileSync } from "node:fs";
import { readFile, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import { renderRerailSchemaModule, SCHEMA_SQL_PATH } from "../scripts/generate-schema-sql.js";
import { openRerailDatabase } from "../src/database.js";
import { getLibraryRule, listLibraryRules } from "../src/rule-library.js";
import { parseConversationRuleDocument } from "../src/rule-schema.js";
import { SqliteRuleStore } from "../src/rule-store.js";
import { MOCK_CONVERSATION_RULES, MOCK_CODE_RULES, MOCK_SESSION_ID, mockSessionId, seedRerailDatabase } from "../src/seed.js";
import { SqliteSessionReader, SqliteSessionStore } from "../src/session-store.js";

const TOOL_CALL_RULES = parseConversationRuleDocument(JSON.parse(readFileSync(resolve("test/fixtures/tool-call-rules.json"), "utf8")));


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
    expect(() => store.upsertCodeRule({ ...codeRule, format: "rerail.policy/v2" })).toThrow(/rerail\.policy\/v1/u);
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
    store.linkWork({ sessionId: "unknown-session", kind: "linear-issue", reference: "ABC-1", url: "https://linear.app/example/issue/ABC-1" }, 11);
    store.linkWork({ sessionId: "unknown-session", kind: "git-branch", reference: "feature/x", url: "https://example.test/tree/feature/x" }, 12);

    const expected = [
      { linkId: 1, sessionId: "unknown-session", kind: "git-branch", reference: "feature/x", url: "https://example.test/tree/feature/x", createdAt: 10 },
      { linkId: 2, sessionId: "unknown-session", kind: "linear-issue", reference: "ABC-1", url: "https://linear.app/example/issue/ABC-1", createdAt: 11 },
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
    const database = openRerailDatabase(path);
    expect(listLibraryRules(database).map(rule => [rule.document.rule.id, rule.document.kind])).toEqual([
      ["mock-cosine-migration-warning", "conversation"],
      ["mock-cosine-prefer-generated-client", "conversation"],
      ["mock-generated-client-imports", "file"],
      ["mock-lazy-query-shaping", "file"],
      ["mock-no-console-log", "file"],
    ]);
    expect(getLibraryRule(database, "mock-no-console-log")?.createdAt).toBe(100_000);
    database.$client.close();
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
