import { execFile, execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, realpathSync, statSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import Database from "better-sqlite3";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { sql } from "drizzle-orm";
import { defaultRerailDatabasePath, openRerailDatabase } from "../src/database.js";
import {
  openScopedRuleCatalog,
  resolveRepositoryContext,
  ScopedRuleCatalog,
} from "../src/scoped-rule-catalog.js";
import { exportRules, exportScopedRules, importScopedRules } from "../src/rule-bootstrap.js";
import { parseConversationRuleDocument } from "../src/rule-schema.js";
import { loadLegacyConversationRuleSources } from "../src/rule-sources.js";
import { SqliteRuleStore } from "../src/rule-store.js";
import { SqliteSessionReader, SqliteSessionStore } from "../src/session-store.js";
import { MOCK_CONVERSATION_RULES, MOCK_CODE_RULES } from "../src/seed.js";
import { ruleDocumentDigest } from "../src/stable-json.js";

const temporaryDirectories: string[] = [];

async function catalogPath(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-scoped-rules-"));
  temporaryDirectories.push(root);
  return join(root, "catalog.sqlite");
}

async function expectCatalogReservation(
  setup: (catalog: ScopedRuleCatalog) => void,
  mutate: (catalog: ScopedRuleCatalog) => void,
  assertMutation: (catalog: ScopedRuleCatalog) => void,
) {
  const path = await catalogPath();
  const repository = { repositoryId: "repo", workspaceRoot: "/repo" };
  const firstDatabase = openRerailDatabase(path);
  const contenderDatabase = openRerailDatabase(path);
  const contender = ScopedRuleCatalog.open(path, repository, contenderDatabase);
  contenderDatabase.run(sql`PRAGMA busy_timeout = 0`);
  const first = ScopedRuleCatalog.open(path, repository, firstDatabase);
  setup(first);
  const originalTransaction = firstDatabase.transaction.bind(firstDatabase);
  let contenderError: unknown;
  let intercepted = false;
  firstDatabase.transaction = (callback, config) => originalTransaction(transaction => {
    if (!intercepted) {
      intercepted = true;
      try {
        contender.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id: "contender" }, { now: 1_001 });
      } catch (error) {
        contenderError = error;
      }
    }
    return callback(transaction);
  }, config);

  try {
    mutate(first);

    expect(intercepted).toBe(true);
    expect(contenderError).toMatchObject({ code: "SQLITE_BUSY" });
    assertMutation(first);
    expect(contender.get("contender")).toBeUndefined();
  } finally {
    first.close();
    contender.close();
  }
}

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

describe("ScopedRuleCatalog", () => {
  it("resolves global rules with project overrides and inherited disabling", async () => {
    const path = await catalogPath();
    const global = ScopedRuleCatalog.open(path, { repositoryId: "repo-a", workspaceRoot: "/repo/a" });
    global.assignRepository("project-a", "Project A", "repo-a", 1_000);
    global.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id: "shared" }, { applicability: "global", now: 1_100 });
    expect(global.setEnabled({ filter: { applicability: "global" } }, false, 1_150)).toBe(1);
    expect(global.get("shared")).toMatchObject({ applicability: "global", enabled: false });
    expect(global.setEnabled({ filter: { applicability: "global" } }, true, 1_175)).toBe(1);

    const project = ScopedRuleCatalog.open(path, { repositoryId: "repo-a", workspaceRoot: "/repo/a-worktree" });
    expect(project.get("shared")).toMatchObject({ applicability: "global", origin: { kind: "global" }, enabled: true });

    project.upsertConversationRule(
      { ...MOCK_CONVERSATION_RULES[0]!, id: "shared", action: { content: "Project override" } },
      { applicability: "project", now: 1_200 },
    );
    expect(project.get("shared")).toMatchObject({
      applicability: "project",
      origin: { kind: "project", projectId: "project-a" },
      enabled: true,
      rule: { action: { content: "Project override" } },
    });
    expect(project.list({ applicability: "global" })).toHaveLength(1);
    expect(project.remove(["shared"], 1_250, "global")).toBe(1);
    expect(project.get("shared")).toMatchObject({ applicability: "project" });
    global.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id: "shared" }, { applicability: "global", now: 1_275 });

    expect(project.setEnabled({ ids: ["shared"] }, false, 1_300)).toBe(1);
    expect(project.conversationRules()).toEqual([]);
    expect(global.get("shared")).toMatchObject({ applicability: "project", enabled: false });
    project.close();
    global.close();
  });

  it("shares one project rule across member repositories without leaking to another", async () => {
    const path = await catalogPath();
    const first = ScopedRuleCatalog.open(path, { repositoryId: "repo-a", workspaceRoot: "/repo/a" });
    first.assignRepository("project-shared", "Shared", "repo-a", 1_000);
    first.assignRepository("project-shared", "Shared", "repo-b", 1_001);
    first.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id: "project-rule" }, { applicability: "project", now: 1_100 });

    const second = ScopedRuleCatalog.open(path, { repositoryId: "repo-b", workspaceRoot: "/repo/b" });
    const unrelated = ScopedRuleCatalog.open(path, { repositoryId: "repo-c", workspaceRoot: "/repo/c" });
    unrelated.assignRepository("project-c", "Unrelated", "repo-c", 1_200);

    expect(second.conversationRules().map(rule => rule.id)).toEqual(["project-rule"]);
    expect(unrelated.conversationRules()).toEqual([]);

    first.upsertConversationRule(
      { ...MOCK_CONVERSATION_RULES[0]!, id: "project-rule", action: { content: "Updated once" } },
      { applicability: "project", now: 1_300 },
    );
    expect(second.get("project-rule")).toMatchObject({ revision: 2, rule: { action: { content: "Updated once" } } });
    expect(unrelated.get("project-rule")).toBeUndefined();

    first.close();
    second.close();
    unrelated.close();
  });

  it("migrates checkout rules into project scope once without changing audit history", async () => {
    const path = await catalogPath();
    const legacyPath = join(temporaryDirectories[0]!, "checkout", ".rerail", "transcripts.sqlite");
    const sessionStore = new SqliteSessionStore(legacyPath);
    sessionStore.openSession({ host: "omp", sessionId: "same-session", cwd: "/repo/a" }, 900);
    sessionStore.append({ sessionId: "same-session", eventId: "event-1", kind: "user-prompt", observedAt: 950, payload: { text: "keep" } });
    sessionStore.close();
    const legacy = new SqliteRuleStore(legacyPath);
    legacy.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id: "legacy" }, { scope: "global", now: 1_000 });
    legacy.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id: "legacy", reference: "edited" }, { now: 2_000 });
    legacy.close();

    const catalog = ScopedRuleCatalog.open(path, { repositoryId: "repo-a", workspaceRoot: "/repo/a" });
    expect(catalog.migrateLegacyRules(legacyPath, 3_000)).toBe(1);
    expect(catalog.migrateLegacyRules(legacyPath, 4_000)).toBe(0);
    expect(catalog.get("legacy")).toMatchObject({
      applicability: "project",
      catalogOrigin: "user",
      createdAt: 1_000,
      updatedAt: 2_000,
      revision: 2,
      rule: { reference: "edited" },
    });
    expect(catalog.remove(["legacy"], 3_500)).toBe(1);
    const secondLegacyPath = join(temporaryDirectories[0]!, "worktree", ".rerail", "transcripts.sqlite");
    const secondLegacy = new SqliteRuleStore(secondLegacyPath);
    secondLegacy.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id: "legacy", reference: "stale copy" }, { now: 1_500 });
    secondLegacy.close();
    expect(catalog.migrateLegacyRules(secondLegacyPath, 4_000)).toBe(0);
    expect(catalog.get("legacy")).toBeUndefined();
    catalog.close();

    const history = new SqliteSessionReader(legacyPath);
    expect(history.sessions()).toEqual([expect.objectContaining({ sessionId: "same-session", eventCount: 1 })]);
    history.close();
  });

  it("round-trips global and project definitions without flattening overrides", async () => {
    const sourcePath = await catalogPath();
    const output = join(temporaryDirectories.at(-1)!, "rules.json");
    const source = ScopedRuleCatalog.open(sourcePath, { repositoryId: "repo-a", workspaceRoot: "/repo/a" });
    source.assignRepository("project-a", "Project A", "repo-a", 1_000);
    source.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id: "shared" }, { applicability: "global", now: 1_100 });
    source.upsertConversationRule(
      { ...MOCK_CONVERSATION_RULES[0]!, id: "shared", reference: "project override" },
      { applicability: "project", enabled: false, now: 1_200 },
    );
    expect(exportScopedRules(source, output, new Date("2026-01-01T00:00:00Z"))).toBe(2);
    source.close();

    const destinationPath = await catalogPath();
    const destination = ScopedRuleCatalog.open(destinationPath, { repositoryId: "repo-b", workspaceRoot: "/repo/b" });
    expect(importScopedRules(destination, output)).toBe(2);
    expect(destination.listOwned()).toHaveLength(2);
    expect(destination.get("shared")).toMatchObject({
      applicability: "project",
      enabled: false,
      createdAt: 1_200,
      updatedAt: 1_200,
      rule: { reference: "project override" },
    });
    destination.close();
  });

  it("stores, exports, and resolves structural tool-call rules", async () => {
    const tools = parseConversationRuleDocument(JSON.parse(readFileSync(resolve("test/fixtures/tool-call-rules.json"), "utf8")))
      .sort((left, right) => left.id.localeCompare(right.id));
    const sourcePath = await catalogPath();
    const output = join(temporaryDirectories.at(-1)!, "rules.json");
    const source = ScopedRuleCatalog.open(sourcePath, { repositoryId: "repo-a", workspaceRoot: "/repo/a" });
    for (const rule of tools) source.upsertConversationRule(rule, { applicability: "global", now: 1_000 });
    expect(source.conversationRules()).toEqual(tools);
    expect(exportScopedRules(source, output, new Date("2026-01-01T00:00:00Z"))).toBe(tools.length);
    source.close();

    const destination = ScopedRuleCatalog.open(await catalogPath(), { repositoryId: "repo-b", workspaceRoot: "/repo/b" });
    expect(importScopedRules(destination, output)).toBe(tools.length);
    expect(destination.conversationRules()).toEqual(tools);
    expect(() => destination.upsertConversationRule({ ...tools[0]!, match: { field: "input.command", regex: "(" } }))
      .toThrow(/regex/u);
    destination.close();
  });

  it("validates scoped imports before atomically changing the catalog and rejects checkout dumps and dumps that predate the current rule contract", async () => {
    const sourcePath = await catalogPath();
    const output = join(temporaryDirectories.at(-1)!, "rules.json");
    const source = ScopedRuleCatalog.open(sourcePath, { repositoryId: "source", workspaceRoot: "/source" });
    source.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id: "first" });
    source.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id: "second" });
    exportScopedRules(source, output);
    source.close();
    const dump = JSON.parse(readFileSync(output, "utf8")) as { rules: Array<{ rule: unknown }> };
    dump.rules[1]!.rule = {};
    writeFileSync(output, JSON.stringify(dump));

    const destinationPath = await catalogPath();
    const destination = ScopedRuleCatalog.open(destinationPath, { repositoryId: "destination", workspaceRoot: "/destination" });
    expect(() => importScopedRules(destination, output)).toThrow();
    expect(destination.list()).toEqual([]);

    const legacyPath = join(temporaryDirectories.at(-1)!, "legacy.sqlite");
    const legacyDump = join(temporaryDirectories.at(-1)!, "legacy.json");
    const legacy = new SqliteRuleStore(legacyPath);
    legacy.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id: "legacy-v1" }, { now: 1_000 });
    exportRules(legacy, legacyDump);
    legacy.close();
    expect(() => importScopedRules(destination, legacyDump)).toThrow(/is a checkout rules dump; only v4 catalog dumps can be imported/u);
    for (const version of [1, 2, 3]) {
      writeFileSync(output, JSON.stringify({ ...dump, version, rules: dump.rules.slice(0, 1) }));
      expect(() => importScopedRules(destination, output))
        .toThrow(new RegExp(`is a v${version} rules dump, which predates the current rule contract; only v4 dumps can be imported`, "u"));
    }
    expect(destination.list()).toEqual([]);
    destination.close();
  });

  it("masks an inherited global rule per project and reveals it after deleting the mask", async () => {
    const path = await catalogPath();
    const global = ScopedRuleCatalog.open(path, { repositoryId: "repo-global", workspaceRoot: "/repo/global" });
    global.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id: "inherited" }, { applicability: "global", now: 1_000 });
    const project = ScopedRuleCatalog.open(path, { repositoryId: "repo-project", workspaceRoot: "/repo/project" });

    expect(project.setEnabled({ ids: ["inherited"] }, false, 2_000)).toBe(1);
    expect(project.get("inherited")).toMatchObject({
      applicability: "project",
      enabled: false,
      origin: { kind: "project", projectId: "repository:repo-project" },
    });
    expect(global.get("inherited")).toMatchObject({ applicability: "global", enabled: true });

    expect(project.remove(["inherited"], 3_000)).toBe(1);
    expect(project.get("inherited")).toMatchObject({ applicability: "global", enabled: true });
    project.close();
    global.close();
  });

  it("rejects toggling a quarantined inherited rule without changing its global definition", async () => {
    const path = await catalogPath();
    const global = ScopedRuleCatalog.open(path, { repositoryId: "repo-global", workspaceRoot: "/repo/global" });
    global.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id: "broken-inherited" }, { applicability: "global", now: 1_000 });
    const raw = new Database(path);
    raw.prepare("UPDATE catalog_rules SET document_json = ? WHERE rule_id = ?").run(
      JSON.stringify({ kind: "conversation", rule: { id: "broken-inherited", unknown: true } }),
      "broken-inherited",
    );
    raw.close();
    const project = ScopedRuleCatalog.open(path, { repositoryId: "repo-project", workspaceRoot: "/repo/project" });

    expect(() => project.setEnabled({ ids: ["broken-inherited"] }, false, 2_000)).toThrow(/quarantined global rule/u);
    expect(global.quarantined()).toEqual([expect.objectContaining({ ruleId: "broken-inherited" })]);
    const stored = new Database(path, { readonly: true });
    expect(stored.prepare("SELECT enabled FROM catalog_rules WHERE rule_id = ? AND owner_kind = 'global'").get("broken-inherited"))
      .toEqual({ enabled: 1 });
    stored.close();
    project.close();
    global.close();
  });

  it("reserves the catalog before direct upsert ownership resolution", async () => {
    await expectCatalogReservation(
      () => {},
      catalog => { catalog.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id: "first" }, { now: 1_000 }); },
      catalog => { expect(catalog.get("first")).toMatchObject({ rule: { id: "first" } }); },
    );
  });

  it("reserves the catalog before restore resolves nested upsert ownership", async () => {
    await expectCatalogReservation(
      () => {},
      catalog => {
        catalog.restore([{ document: { kind: "conversation", rule: { ...MOCK_CONVERSATION_RULES[0]!, id: "restored" } } }]);
      },
      catalog => { expect(catalog.get("restored")).toMatchObject({ rule: { id: "restored" } }); },
    );
  });

  it("reserves the catalog before inherited-rule masking resolves nested upsert ownership", async () => {
    await expectCatalogReservation(
      catalog => {
        catalog.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id: "inherited" }, { applicability: "global", now: 1_000 });
      },
      catalog => { catalog.setEnabled({ ids: ["inherited"] }, false, 2_000); },
      catalog => {
        expect(catalog.get("inherited")).toMatchObject({ applicability: "project", enabled: false });
      },
    );
  });

  it("serves code rule and conversation rule projections, filters, and external data-version changes", async () => {
    const path = await catalogPath();
    const reader = ScopedRuleCatalog.open(path, { repositoryId: "repo", workspaceRoot: "/repo" });
    const writer = ScopedRuleCatalog.open(path, { repositoryId: "repo", workspaceRoot: "/repo" });
    const initialVersion = reader.dataVersion();
    writer.upsertDocument({ kind: "file", rule: MOCK_CODE_RULES[0]! }, { now: 1_000 });
    writer.upsertDocument(
      {
        kind: "conversation",
        rule: {
          ...MOCK_CONVERSATION_RULES[0]!,
          id: "conversation-filter",
          reference: "Generated API client",
          sample: { source: "assistant-text", boundary: "sentence" },
          action: { content: "Use the generated client." },
        },
      },
      { enabled: false, now: 2_000 },
    );

    expect(reader.dataVersion()).not.toBe(initialVersion);
    expect(reader.codeRules().map(rule => rule.id)).toEqual([MOCK_CODE_RULES[0]!.id]);
    expect(reader.conversationRuleIds()).toEqual(new Set(["conversation-filter"]));
    expect(reader.conversationRules()).toEqual([]);
    expect(reader.list({
      kind: "conversation",
      enabled: false,
      text: "generated",
      source: "assistant-text",
      boundary: "sentence",
      mode: "steer",
    })).toEqual([expect.objectContaining({ rule: expect.objectContaining({ id: "conversation-filter" }) })]);
    expect(reader.list({ mode: "steer" }).map(stored => stored.rule.id)).toEqual(["conversation-filter", MOCK_CODE_RULES[0]!.id]);
    expect(reader.list({ mode: "log" })).toEqual([]);
    reader.close();
    writer.close();
  });

  it("keeps local seed edits while refreshing unchanged bundled definitions", async () => {
    const path = await catalogPath();
    const catalog = ScopedRuleCatalog.open(path, { repositoryId: "repo", workspaceRoot: "/repo" });
    const bundled = { kind: "conversation" as const, rule: { ...MOCK_CONVERSATION_RULES[0]!, id: "bundled" } };
    const refreshed = {
      kind: "conversation" as const,
      rule: { ...bundled.rule, reference: "Updated bundled reference" },
    };

    expect(catalog.seed([bundled], 1_000)).toEqual({ inserted: 1, refreshed: 0, kept: 0 });
    expect(catalog.seed([bundled], 2_000)).toEqual({ inserted: 0, refreshed: 0, kept: 1 });
    expect(catalog.seed([refreshed], 3_000)).toEqual({ inserted: 0, refreshed: 1, kept: 0 });
    catalog.upsertConversationRule({ ...refreshed.rule, reference: "Local edit" }, { now: 4_000 });
    expect(catalog.seed([bundled], 5_000)).toEqual({ inserted: 0, refreshed: 0, kept: 1 });
    expect(catalog.get("bundled")).toMatchObject({ rule: { reference: "Local edit" } });
    catalog.close();
  });

  it("seeds an opt-in document disabled without storing its seeding flag, and keeps the user's enabled state on refresh", async () => {
    const path = await catalogPath();
    const catalog = ScopedRuleCatalog.open(path, { repositoryId: "repo", workspaceRoot: "/repo" });
    const rule = { ...MOCK_CONVERSATION_RULES[0]!, id: "opt-in" };
    const optIn = { kind: "conversation" as const, rule, enabledByDefault: false };

    expect(catalog.seed([optIn], 1_000)).toEqual({ inserted: 1, refreshed: 0, kept: 0 });
    expect(catalog.get("opt-in")).toMatchObject({ enabled: false, sourceDigest: ruleDocumentDigest({ kind: "conversation", rule }) });
    expect(catalog.get("opt-in")).not.toHaveProperty("enabledByDefault");
    expect(catalog.conversationRules()).toEqual([]);
    catalog.setEnabled({ ids: ["opt-in"] }, true, 1_500);
    expect(catalog.seed([{ ...optIn, rule: { ...rule, reference: "Updated" } }], 2_000)).toEqual({ inserted: 0, refreshed: 1, kept: 0 });
    expect(catalog.get("opt-in")).toMatchObject({ enabled: true, rule: { reference: "Updated" } });
    catalog.close();
  });

  it("never reads a legacy source again once it was imported, so a retired format cannot break startup", async () => {
    const path = await catalogPath();
    const root = temporaryDirectories.at(-1)!;
    const catalog = ScopedRuleCatalog.open(path, { repositoryId: "repo", workspaceRoot: root });
    const rulesFile = join(root, "conversation-rules.json");
    const document = (rule: unknown) => JSON.stringify({ version: 1, rules: [rule] });
    const { mode: _mode, ...legacyRule } = { ...MOCK_CONVERSATION_RULES[0]!, id: "file-rule", action: { delivery: "steer", content: "Old shape" } };
    writeFileSync(rulesFile, document({ ...MOCK_CONVERSATION_RULES[0]!, id: "file-rule" }));
    const [source] = loadLegacyConversationRuleSources({}, root, "conversation-rules.json");
    expect(catalog.migrateConversationRules(source!.path, source!.rules, 1_000)).toBe(1);

    writeFileSync(rulesFile, document(legacyRule));
    const [imported] = loadLegacyConversationRuleSources({}, root, "conversation-rules.json");
    expect(catalog.migrateConversationRules(imported!.path, imported!.rules, 2_000)).toBe(0);
    expect(catalog.get("file-rule")?.rule.mode).toBe("steer");

    const otherFile = join(root, "other-rules.json");
    writeFileSync(otherFile, document(legacyRule));
    const [fresh] = loadLegacyConversationRuleSources({ ruleFiles: [otherFile] }, root, "conversation-rules.json");
    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    try {
      // A file in the retired shape that was never imported is skipped, not fatal, and stays unrecorded.
      expect(catalog.migrateConversationRules(fresh!.path, fresh!.rules, 3_000)).toBe(0);
      expect(stderr.mock.calls.map(([line]) => String(line))).toContainEqual(
        expect.stringMatching(/^rerail: skipped rule file \S+other-rules\.json: it was written before rule modes; /u),
      );
    } finally {
      stderr.mockRestore();
    }
    // Any other invalid file still fails, as it always did.
    writeFileSync(otherFile, JSON.stringify({ version: 1, rules: "not-an-array" }));
    const [malformed] = loadLegacyConversationRuleSources({ ruleFiles: [otherFile] }, root, "conversation-rules.json");
    expect(() => catalog.migrateConversationRules(malformed!.path, malformed!.rules, 3_200)).toThrow("rule document.rules must be an array");
    writeFileSync(otherFile, document({ ...MOCK_CONVERSATION_RULES[0]!, id: "fixed-rule" }));
    const [fixed] = loadLegacyConversationRuleSources({ ruleFiles: [otherFile] }, root, "conversation-rules.json");
    expect(catalog.migrateConversationRules(fixed!.path, fixed!.rules, 3_500)).toBe(1);

    const legacyPath = join(root, "legacy.sqlite");
    const legacy = new SqliteRuleStore(legacyPath);
    legacy.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id: "legacy-db" }, { now: 1_000 });
    legacy.close();
    expect(catalog.migrateLegacyRules(legacyPath, 4_000)).toBe(1);
    await rm(legacyPath);
    writeFileSync(legacyPath, "not a database");
    expect(catalog.migrateLegacyRules(legacyPath, 5_000)).toBe(0);
    catalog.close();
  });

  it("opens the Rerail database and imports a checkout's pre-catalog rules once, leaving its history behind", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-scoped-open-"));
    temporaryDirectories.push(root);
    const legacyPath = join(root, ".rerail", "transcripts.sqlite");
    const history = new SqliteSessionStore(legacyPath);
    history.openSession({ host: "omp", sessionId: "old-session", cwd: root }, 500);
    history.close();
    const legacy = new SqliteRuleStore(legacyPath);
    legacy.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id: "legacy-open" }, { now: 1_000 });
    legacy.close();

    const home = join(root, "home");
    expect(defaultRerailDatabasePath({ RERAIL_HOME: home })).toBe(join(home, "rerail.sqlite"));
    const previousHome = process.env.RERAIL_HOME;
    process.env.RERAIL_HOME = home;
    try {
      const catalog = openScopedRuleCatalog(root);
      expect(catalog.path).toBe(join(home, "rerail.sqlite"));
      expect(catalog.get("legacy-open")).toMatchObject({ applicability: "project", catalogOrigin: "user" });
      expect(catalog.migrateCheckoutRules()).toBe(0);
      expect(catalog.migrateConversationRules(join(root, "host-rules.json"), [
        { ...MOCK_CONVERSATION_RULES[0]!, id: "host-rule" },
      ], 2_000)).toBe(1);
      expect(catalog.migrateConversationRules(join(root, "host-rules.json"), [
        { ...MOCK_CONVERSATION_RULES[0]!, id: "host-rule" },
      ], 3_000)).toBe(0);
      catalog.close();
      // The checkout's history is not copied; the old file keeps it.
      expect(new SqliteSessionReader(join(home, "rerail.sqlite")).sessions()).toEqual([]);
      expect(new SqliteSessionReader(legacyPath).sessions()).toEqual([expect.objectContaining({ sessionId: "old-session" })]);
    } finally {
      if (previousHome === undefined) delete process.env.RERAIL_HOME;
      else process.env.RERAIL_HOME = previousHome;
    }
  });

  it("imports the checkout's pre-catalog rules whenever a catalog is opened for an explicit repository", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-scoped-open-"));
    temporaryDirectories.push(root);
    const legacy = new SqliteRuleStore(join(root, ".rerail", "transcripts.sqlite"));
    legacy.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id: "legacy-explicit" }, { now: 1_000 });
    legacy.close();

    const catalog = ScopedRuleCatalog.open(join(root, "home", "rerail.sqlite"), { repositoryId: "repo", workspaceRoot: root });
    expect(catalog.get("legacy-explicit")).toMatchObject({ applicability: "project", catalogOrigin: "user" });
    catalog.close();
  });

  it("quarantines an unreadable row without blocking its removal", async () => {
    const path = await catalogPath();
    const catalog = ScopedRuleCatalog.open(path, { repositoryId: "repo", workspaceRoot: "/repo" });
    catalog.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id: "healthy" }, { now: 1_000 });
    catalog.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id: "broken" }, { now: 1_000 });
    const database = new Database(path);
    database.prepare("UPDATE catalog_rules SET document_json = ? WHERE rule_id = ?")
      .run(JSON.stringify({ kind: "conversation", rule: { id: "broken", unknown: true } }), "broken");
    database.close();

    expect(catalog.list().map(({ rule }) => rule.id)).toEqual(["healthy"]);
    expect(catalog.quarantined()).toEqual([expect.objectContaining({ ruleId: "broken" })]);
    expect(catalog.remove(["broken"])).toBe(1);
    expect(catalog.quarantined()).toEqual([]);
    catalog.close();
  });
});

describe("rules.sqlite adoption", () => {
  /** A machine catalog as the two-file release left it: floor v10, revision 12, thousands of rules, several projects. */
  async function legacyCatalog(home: string, ruleCount: number): Promise<string> {
    const path = join(home, "rules.sqlite");
    const repositories = ["repo-a", "repo-b", "repo-c"].map(repositoryId => ({ repositoryId, workspaceRoot: `/${repositoryId}` }));
    const catalogs = repositories.map(repository => ScopedRuleCatalog.open(path, repository));
    const rule = MOCK_CONVERSATION_RULES[0]!;
    catalogs[0]!.restore(Array.from({ length: ruleCount }, (_value, index) => ({
      document: { kind: "conversation" as const, rule: { ...rule, id: `rule-${index}` } },
      options: { applicability: index % 2 === 0 ? "global" as const : "project" as const, now: 1_000 + index },
    })));
    catalogs[1]!.upsertConversationRule({ ...rule, id: "project-b-only" }, { now: 5_000 });
    catalogs[2]!.upsertConversationRule({ ...rule, id: "rule-1", reference: "c override" }, { now: 6_000 });
    for (const catalog of catalogs) catalog.close();
    const database = new Database(path);
    database.exec([
      "DROP INDEX sessions_repository_started",
      "ALTER TABLE sessions DROP COLUMN repository_id",
      "UPDATE rerail_schema_revision SET revision = 12",
      "PRAGMA user_version = 10",
    ].join(";"));
    database.close();
    return path;
  }

  function counts(path: string): Record<string, number> {
    const database = new Database(path, { readonly: true });
    try {
      return Object.fromEntries(["catalog_rules", "rule_projects", "rule_repositories", "rule_project_repositories", "rule_catalog_identities"]
        .map(table => [table, (database.prepare(`SELECT count(*) AS n FROM ${table}`).get() as { n: number }).n]));
    } finally {
      database.close();
    }
  }

  it("copies every rule into rerail.sqlite on first open, leaves rules.sqlite untouched, and never copies again", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-adopt-"));
    temporaryDirectories.push(home);
    const legacyPath = await legacyCatalog(home, 3_683);
    const before = counts(legacyPath);
    expect(before.catalog_rules).toBe(3_685);
    const legacyBytes = readFileSync(legacyPath);
    const legacyModified = statSync(legacyPath).mtimeMs;
    const path = join(home, "rerail.sqlite");
    expect(existsSync(path)).toBe(false);

    const catalog = ScopedRuleCatalog.open(path, { repositoryId: "repo-c", workspaceRoot: "/repo-c" });
    expect(counts(path)).toEqual(before);
    expect(catalog.get("rule-1")).toMatchObject({ applicability: "project", rule: { reference: "c override" } });
    expect(catalog.get("rule-0")).toMatchObject({ applicability: "global" });
    const store = new SqliteSessionStore(path, { repositoryId: "repo-c" });
    store.openSession({ host: "omp", sessionId: "after-adoption", cwd: "/repo-c" });
    store.close();
    catalog.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id: "written-after" });
    catalog.close();
    expect(readFileSync(legacyPath).equals(legacyBytes)).toBe(true);
    expect(statSync(legacyPath).mtimeMs).toBe(legacyModified);

    // A later change to the old file is never re-adopted: rerail.sqlite is authoritative once it exists.
    const stale = ScopedRuleCatalog.open(legacyPath, { repositoryId: "repo-a", workspaceRoot: "/repo-a" });
    stale.upsertConversationRule({ ...MOCK_CONVERSATION_RULES[0]!, id: "stale-write" });
    stale.close();
    const reopened = ScopedRuleCatalog.open(path, { repositoryId: "repo-a", workspaceRoot: "/repo-a" });
    expect(reopened.get("stale-write")).toBeUndefined();
    expect(reopened.get("written-after")).toBeUndefined();
    expect(reopened.get("project-b-only")).toBeUndefined();
    expect(counts(path).catalog_rules).toBe(before.catalog_rules! + 1);
    reopened.close();
    const reader = new SqliteSessionReader(path, { repositoryId: "repo-c" });
    expect(reader.sessions().map(session => session.sessionId)).toEqual(["after-adoption"]);
    reader.close();
  });

  it("agrees on one copy when several processes open the database at once", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-adopt-race-"));
    temporaryDirectories.push(home);
    const legacyPath = await legacyCatalog(home, 200);
    const path = join(home, "rerail.sqlite");
    const script = [
      `import { openRerailDatabase } from ${JSON.stringify(resolve("src/database.ts"))};`,
      `openRerailDatabase(${JSON.stringify(path)}).$client.close();`,
    ].join("\n");
    await Promise.all(Array.from({ length: 6 }, () => new Promise<void>((resolveRun, reject) => {
      const child = execFile(process.execPath, ["--import", "tsx", "--input-type=module", "-e", script], error => error ? reject(error) : resolveRun());
      child.stdin?.end();
    })));
    expect(counts(path)).toEqual(counts(legacyPath));
    expect(readdirSync(home).filter(name => name.includes(".adopt"))).toEqual([]);
  }, 60_000);

  it("removes a partial copy when VACUUM INTO fails", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-adopt-fail-"));
    temporaryDirectories.push(home);
    await legacyCatalog(home, 10);
    const path = join(home, "rerail.sqlite");
    const prepare = Database.prototype.prepare;
    const spy = vi.spyOn(Database.prototype, "prepare").mockImplementation(function (this: Database.Database, source: string) {
      if (source !== "VACUUM INTO ?") return prepare.call(this, source);
      return { run: (target: string) => {
        writeFileSync(target, "partial");
        throw new Error("database or disk is full");
      } } as unknown as Database.Statement;
    });
    try {
      expect(() => openRerailDatabase(path)).toThrow("database or disk is full");
    } finally {
      spy.mockRestore();
    }
    expect(readdirSync(home).filter(name => name.includes(".adopt"))).toEqual([]);
    expect(existsSync(path)).toBe(false);
  });
});

describe("repository identity", () => {
  it("shares a stable id across worktrees without assigning clones the same identity", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-repository-id-"));
    temporaryDirectories.push(root);
    const repository = join(root, "repository");
    const worktree = join(root, "worktree");
    const clone = join(root, "clone");
    const replacementSource = join(root, "replacement-source");
    execFileSync("git", ["init", repository]);
    execFileSync("git", ["-C", repository, "config", "user.email", "rerail@example.test"]);
    execFileSync("git", ["-C", repository, "config", "user.name", "Rerail Test"]);
    execFileSync("git", ["-C", repository, "commit", "--allow-empty", "-m", "initial"]);
    execFileSync("git", ["-C", repository, "worktree", "add", worktree]);
    execFileSync("git", ["clone", repository, clone]);

    const main = resolveRepositoryContext(repository);
    const linked = resolveRepositoryContext(worktree);
    const copied = resolveRepositoryContext(clone);

    expect(linked.repositoryId).toBe(main.repositoryId);
    expect(linked.workspaceRoot).toBe(realpathSync(worktree));
    expect(copied.repositoryId).not.toBe(main.repositoryId);

    await rm(clone, { recursive: true, force: true });
    execFileSync("git", ["init", replacementSource]);
    execFileSync("git", ["-C", replacementSource, "config", "user.email", "rerail@example.test"]);
    execFileSync("git", ["-C", replacementSource, "config", "user.name", "Rerail Test"]);
    execFileSync("git", ["-C", replacementSource, "commit", "--allow-empty", "-m", "replacement"]);
    execFileSync("git", ["clone", replacementSource, clone]);
    expect(resolveRepositoryContext(clone).repositoryId).not.toBe(copied.repositoryId);
  });
});
