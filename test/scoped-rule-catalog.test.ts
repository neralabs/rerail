import { execFileSync } from "node:child_process";
import { readFileSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import Database from "better-sqlite3";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { sql } from "drizzle-orm";
import { defaultRerailDatabasePath, openRerailDatabase } from "../src/database.js";
import {
  openScopedRuleCatalog,
  resolveRepositoryContext,
  ScopedRuleCatalog,
} from "../src/scoped-rule-catalog.js";
import { exportScopedRules, importScopedRules } from "../src/rule-bootstrap.js";
import { parseCosineRuleDocument } from "../src/rule-schema.js";
import { MOCK_COSINE_RULES, MOCK_POLICY_RULES } from "../src/seed.js";
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
        contender.upsertCosine({ ...MOCK_COSINE_RULES[0]!, id: "contender" }, { now: 1_001 });
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
    global.upsertCosine({ ...MOCK_COSINE_RULES[0]!, id: "shared" }, { applicability: "global", now: 1_100 });
    expect(global.setEnabled({ filter: { applicability: "global" } }, false, 1_150)).toBe(1);
    expect(global.get("shared")).toMatchObject({ applicability: "global", enabled: false });
    expect(global.setEnabled({ filter: { applicability: "global" } }, true, 1_175)).toBe(1);

    const project = ScopedRuleCatalog.open(path, { repositoryId: "repo-a", workspaceRoot: "/repo/a-worktree" });
    expect(project.get("shared")).toMatchObject({ applicability: "global", origin: { kind: "global" }, enabled: true });

    project.upsertCosine(
      { ...MOCK_COSINE_RULES[0]!, id: "shared", action: { delivery: "steer", content: "Project override" } },
      { applicability: "project", now: 1_200 },
    );
    expect(project.get("shared")).toMatchObject({
      applicability: "project",
      origin: { kind: "project", projectId: "project-a" },
      enabled: true,
      rule: { action: { content: "Project override" } },
    });
    expect(project.list({ applicability: "global" })).toHaveLength(1);
    expect(project.remove(["shared"], "global")).toBe(1);
    expect(project.get("shared")).toMatchObject({ applicability: "project" });
    global.upsertCosine({ ...MOCK_COSINE_RULES[0]!, id: "shared" }, { applicability: "global", now: 1_275 });

    expect(project.setEnabled({ ids: ["shared"] }, false, 1_300)).toBe(1);
    expect(project.cosineRules()).toEqual([]);
    expect(global.get("shared")).toMatchObject({ applicability: "project", enabled: false });
    project.close();
    global.close();
  });

  it("shares one project rule across member repositories without leaking to another", async () => {
    const path = await catalogPath();
    const first = ScopedRuleCatalog.open(path, { repositoryId: "repo-a", workspaceRoot: "/repo/a" });
    first.assignRepository("project-shared", "Shared", "repo-a", 1_000);
    first.assignRepository("project-shared", "Shared", "repo-b", 1_001);
    first.upsertCosine({ ...MOCK_COSINE_RULES[0]!, id: "project-rule" }, { applicability: "project", now: 1_100 });

    const second = ScopedRuleCatalog.open(path, { repositoryId: "repo-b", workspaceRoot: "/repo/b" });
    const unrelated = ScopedRuleCatalog.open(path, { repositoryId: "repo-c", workspaceRoot: "/repo/c" });
    unrelated.assignRepository("project-c", "Unrelated", "repo-c", 1_200);

    expect(second.cosineRules().map(rule => rule.id)).toEqual(["project-rule"]);
    expect(unrelated.cosineRules()).toEqual([]);

    first.upsertCosine(
      { ...MOCK_COSINE_RULES[0]!, id: "project-rule", action: { delivery: "steer", content: "Updated once" } },
      { applicability: "project", now: 1_300 },
    );
    expect(second.get("project-rule")).toMatchObject({ revision: 2, rule: { action: { content: "Updated once" } } });
    expect(unrelated.get("project-rule")).toBeUndefined();

    first.close();
    second.close();
    unrelated.close();
  });

  it("round-trips global and project definitions without flattening overrides", async () => {
    const sourcePath = await catalogPath();
    const output = join(temporaryDirectories.at(-1)!, "rules.json");
    const source = ScopedRuleCatalog.open(sourcePath, { repositoryId: "repo-a", workspaceRoot: "/repo/a" });
    source.assignRepository("project-a", "Project A", "repo-a", 1_000);
    source.upsertCosine({ ...MOCK_COSINE_RULES[0]!, id: "shared" }, { applicability: "global", now: 1_100 });
    source.upsertCosine(
      { ...MOCK_COSINE_RULES[0]!, id: "shared", reference: "project override" },
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
    const tools = parseCosineRuleDocument(JSON.parse(readFileSync(resolve("test/fixtures/tool-call-rules.json"), "utf8")))
      .sort((left, right) => left.id.localeCompare(right.id));
    const sourcePath = await catalogPath();
    const output = join(temporaryDirectories.at(-1)!, "rules.json");
    const source = ScopedRuleCatalog.open(sourcePath, { repositoryId: "repo-a", workspaceRoot: "/repo/a" });
    for (const rule of tools) source.upsertCosine(rule, { applicability: "global", now: 1_000 });
    expect(source.cosineRules()).toEqual(tools);
    expect(exportScopedRules(source, output, new Date("2026-01-01T00:00:00Z"))).toBe(tools.length);
    source.close();

    const destination = ScopedRuleCatalog.open(await catalogPath(), { repositoryId: "repo-b", workspaceRoot: "/repo/b" });
    expect(importScopedRules(destination, output)).toBe(tools.length);
    expect(destination.cosineRules()).toEqual(tools);
    expect(() => destination.upsertCosine({ ...tools[0]!, match: { field: "input.command", regex: "(" } }))
      .toThrow(/regex/u);
    destination.close();
  });

  it("validates scoped imports before atomically changing the catalog", async () => {
    const sourcePath = await catalogPath();
    const output = join(temporaryDirectories.at(-1)!, "rules.json");
    const source = ScopedRuleCatalog.open(sourcePath, { repositoryId: "source", workspaceRoot: "/source" });
    source.upsertCosine({ ...MOCK_COSINE_RULES[0]!, id: "first" });
    source.upsertCosine({ ...MOCK_COSINE_RULES[0]!, id: "second" });
    exportScopedRules(source, output);
    source.close();
    const dump = JSON.parse(readFileSync(output, "utf8")) as { rules: Array<{ rule: unknown }> };
    dump.rules[1]!.rule = {};
    writeFileSync(output, JSON.stringify(dump));

    const destinationPath = await catalogPath();
    const destination = ScopedRuleCatalog.open(destinationPath, { repositoryId: "destination", workspaceRoot: "/destination" });
    expect(() => importScopedRules(destination, output)).toThrow();
    expect(destination.list()).toEqual([]);
    destination.close();
  });

  it("masks an inherited global rule per project and reveals it after deleting the mask", async () => {
    const path = await catalogPath();
    const global = ScopedRuleCatalog.open(path, { repositoryId: "repo-global", workspaceRoot: "/repo/global" });
    global.upsertCosine({ ...MOCK_COSINE_RULES[0]!, id: "inherited" }, { applicability: "global", now: 1_000 });
    const project = ScopedRuleCatalog.open(path, { repositoryId: "repo-project", workspaceRoot: "/repo/project" });

    expect(project.setEnabled({ ids: ["inherited"] }, false, 2_000)).toBe(1);
    expect(project.get("inherited")).toMatchObject({
      applicability: "project",
      enabled: false,
      origin: { kind: "project", projectId: "repository:repo-project" },
    });
    expect(global.get("inherited")).toMatchObject({ applicability: "global", enabled: true });

    expect(project.remove(["inherited"])).toBe(1);
    expect(project.get("inherited")).toMatchObject({ applicability: "global", enabled: true });
    project.close();
    global.close();
  });

  it("rejects toggling a quarantined inherited rule without changing its global definition", async () => {
    const path = await catalogPath();
    const global = ScopedRuleCatalog.open(path, { repositoryId: "repo-global", workspaceRoot: "/repo/global" });
    global.upsertCosine({ ...MOCK_COSINE_RULES[0]!, id: "broken-inherited" }, { applicability: "global", now: 1_000 });
    const raw = new Database(path);
    raw.prepare("UPDATE catalog_rules SET document_json = ? WHERE rule_id = ?").run(
      JSON.stringify({ kind: "cosine", rule: { id: "broken-inherited", unknown: true } }),
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
      catalog => { catalog.upsertCosine({ ...MOCK_COSINE_RULES[0]!, id: "first" }, { now: 1_000 }); },
      catalog => { expect(catalog.get("first")).toMatchObject({ rule: { id: "first" } }); },
    );
  });

  it("reserves the catalog before restore resolves nested upsert ownership", async () => {
    await expectCatalogReservation(
      () => {},
      catalog => {
        catalog.restore([{ document: { kind: "cosine", rule: { ...MOCK_COSINE_RULES[0]!, id: "restored" } } }]);
      },
      catalog => { expect(catalog.get("restored")).toMatchObject({ rule: { id: "restored" } }); },
    );
  });

  it("reserves the catalog before inherited-rule masking resolves nested upsert ownership", async () => {
    await expectCatalogReservation(
      catalog => {
        catalog.upsertCosine({ ...MOCK_COSINE_RULES[0]!, id: "inherited" }, { applicability: "global", now: 1_000 });
      },
      catalog => { catalog.setEnabled({ ids: ["inherited"] }, false, 2_000); },
      catalog => {
        expect(catalog.get("inherited")).toMatchObject({ applicability: "project", enabled: false });
      },
    );
  });

  it("serves policy and cosine projections, filters, and external data-version changes", async () => {
    const path = await catalogPath();
    const reader = ScopedRuleCatalog.open(path, { repositoryId: "repo", workspaceRoot: "/repo" });
    const writer = ScopedRuleCatalog.open(path, { repositoryId: "repo", workspaceRoot: "/repo" });
    const initialVersion = reader.dataVersion();
    writer.upsertDocument({ kind: "policy", rule: MOCK_POLICY_RULES[0]! }, { now: 1_000 });
    writer.upsertDocument(
      {
        kind: "cosine",
        rule: {
          ...MOCK_COSINE_RULES[0]!,
          id: "cosine-filter",
          reference: "Generated API client",
          sample: { source: "assistant-text", boundary: "sentence" },
          action: { delivery: "steer", content: "Use the generated client." },
        },
      },
      { enabled: false, now: 2_000 },
    );

    expect(reader.dataVersion()).not.toBe(initialVersion);
    expect(reader.policyRules().map(rule => rule.id)).toEqual([MOCK_POLICY_RULES[0]!.id]);
    expect(reader.cosineIds()).toEqual(new Set(["cosine-filter"]));
    expect(reader.cosineRules()).toEqual([]);
    expect(reader.list({
      kind: "cosine",
      enabled: false,
      text: "generated",
      source: "assistant-text",
      boundary: "sentence",
      delivery: "steer",
    })).toEqual([expect.objectContaining({ rule: expect.objectContaining({ id: "cosine-filter" }) })]);
    reader.close();
    writer.close();
  });

  it("keeps local seed edits while refreshing unchanged bundled definitions", async () => {
    const path = await catalogPath();
    const catalog = ScopedRuleCatalog.open(path, { repositoryId: "repo", workspaceRoot: "/repo" });
    const bundled = { kind: "cosine" as const, rule: { ...MOCK_COSINE_RULES[0]!, id: "bundled" } };
    const refreshed = {
      kind: "cosine" as const,
      rule: { ...bundled.rule, reference: "Updated bundled reference" },
    };

    expect(catalog.seed([bundled], 1_000)).toEqual({ inserted: 1, refreshed: 0, kept: 0 });
    expect(catalog.seed([bundled], 2_000)).toEqual({ inserted: 0, refreshed: 0, kept: 1 });
    expect(catalog.seed([refreshed], 3_000)).toEqual({ inserted: 0, refreshed: 1, kept: 0 });
    catalog.upsertCosine({ ...refreshed.rule, reference: "Local edit" }, { now: 4_000 });
    expect(catalog.seed([bundled], 5_000)).toEqual({ inserted: 0, refreshed: 0, kept: 1 });
    expect(catalog.get("bundled")).toMatchObject({ rule: { reference: "Local edit" } });
    catalog.close();
  });

  it("seeds an opt-in document disabled without storing its seeding flag, and keeps the user's enabled state on refresh", async () => {
    const path = await catalogPath();
    const catalog = ScopedRuleCatalog.open(path, { repositoryId: "repo", workspaceRoot: "/repo" });
    const rule = { ...MOCK_COSINE_RULES[0]!, id: "opt-in" };
    const optIn = { kind: "cosine" as const, rule, enabledByDefault: false };

    expect(catalog.seed([optIn], 1_000)).toEqual({ inserted: 1, refreshed: 0, kept: 0 });
    expect(catalog.get("opt-in")).toMatchObject({ enabled: false, sourceDigest: ruleDocumentDigest({ kind: "cosine", rule }) });
    expect(catalog.get("opt-in")).not.toHaveProperty("enabledByDefault");
    expect(catalog.cosineRules()).toEqual([]);
    catalog.setEnabled({ ids: ["opt-in"] }, true, 1_500);
    expect(catalog.seed([{ ...optIn, rule: { ...rule, reference: "Updated" } }], 2_000)).toEqual({ inserted: 0, refreshed: 1, kept: 0 });
    expect(catalog.get("opt-in")).toMatchObject({ enabled: true, rule: { reference: "Updated" } });
    catalog.close();
  });

  it("opens the workspace's catalog in the Rerail home's database", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-scoped-open-"));
    temporaryDirectories.push(root);
    const home = join(root, "home");
    expect(defaultRerailDatabasePath({ RERAIL_HOME: home })).toBe(join(home, "rerail.sqlite"));
    const previousHome = process.env.RERAIL_HOME;
    process.env.RERAIL_HOME = home;
    try {
      const catalog = openScopedRuleCatalog(root);
      expect(catalog.path).toBe(join(home, "rerail.sqlite"));
      catalog.close();
    } finally {
      if (previousHome === undefined) delete process.env.RERAIL_HOME;
      else process.env.RERAIL_HOME = previousHome;
    }
  });

  it("quarantines an unreadable row without blocking its removal", async () => {
    const path = await catalogPath();
    const catalog = ScopedRuleCatalog.open(path, { repositoryId: "repo", workspaceRoot: "/repo" });
    catalog.upsertCosine({ ...MOCK_COSINE_RULES[0]!, id: "healthy" }, { now: 1_000 });
    catalog.upsertCosine({ ...MOCK_COSINE_RULES[0]!, id: "broken" }, { now: 1_000 });
    const database = new Database(path);
    database.prepare("UPDATE catalog_rules SET document_json = ? WHERE rule_id = ?")
      .run(JSON.stringify({ kind: "cosine", rule: { id: "broken", unknown: true } }), "broken");
    database.close();

    expect(catalog.list().map(({ rule }) => rule.id)).toEqual(["healthy"]);
    expect(catalog.quarantined()).toEqual([expect.objectContaining({ ruleId: "broken" })]);
    expect(catalog.remove(["broken"])).toBe(1);
    expect(catalog.quarantined()).toEqual([]);
    catalog.close();
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
