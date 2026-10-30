import { describe, expect, it } from "vitest";
import { exportRulesDump, importRulesDump } from "../../../src/engine/catalog/rule-bootstrap.js";
import {
  addAttachment,
  loadCheckoutRuleSet,
  setRuleSetting,
  setUpRepository,
} from "../../../src/engine/catalog/rule-configuration.js";
import {
  copyRerailRuleToLibrary,
  getLibraryRule,
  listLibraryRules,
  quarantinedLibraryRules,
  removeLibraryRule,
  upsertLibraryRule,
  writeLibraryRule,
} from "../../../src/engine/catalog/rule-library.js";
import { libraryRules, repositories, ruleAttachments, ruleSettings } from "../../../src/engine/storage/schema.js";
import {
  addRepository,
  checkout,
  conversation,
  codeRule,
  REPOSITORY,
  RUNNING,
  temporaryDatabase,
  testBundle,
} from "../../support/rule-configuration-builders.js";

const ruleCatalog = testBundle();
const CHECKOUT = "/work/app";

async function repositoryWithLintPack(folders: readonly string[]) {
  const db = await temporaryDatabase();
  addRepository(db, REPOSITORY);
  setUpRepository(db, REPOSITORY, { packs: folders.map(folder => ({ packId: "lint", folder })) });
  return db;
}

describe("library rules", () => {
  it("rejects a By Rerail rule id and keeps one kind per id", async () => {
    const db = await temporaryDatabase();
    expect(() => upsertLibraryRule(db, ruleCatalog, codeRule("no-console"), "code")).toThrow(/By Rerail rule id/u);

    upsertLibraryRule(db, ruleCatalog, codeRule("mine"), "code");
    expect(() => upsertLibraryRule(db, ruleCatalog, conversation("mine"), "conversation")).toThrow(/exists with kind code/u);
    expect(getLibraryRule(db, "mine")?.document.kind).toBe("code");
  });

  it("bumps a conversation rule's revision on each write and removes everything that points at a rule", async () => {
    const db = await repositoryWithLintPack([""]);
    upsertLibraryRule(db, ruleCatalog, conversation("chat"), "conversation");
    expect(upsertLibraryRule(db, ruleCatalog, conversation("chat"), "conversation").revision).toBe(2);
    addAttachment(db, { repositoryKey: REPOSITORY, folder: "api", sourceKind: "library_rule", sourceId: "chat" });
    setRuleSetting(db, { owner: { kind: "repository", repositoryKey: REPOSITORY }, folder: "api", targetKind: "rule", targetId: "chat", on: false, mode: null });

    expect(removeLibraryRule(db, "chat")).toBe(true);
    expect(db.select().from(ruleAttachments).all().map(row => row.sourceKind)).toEqual(["pack"]);
    expect(db.select().from(ruleSettings).all()).toEqual([]);
    upsertLibraryRule(db, ruleCatalog, codeRule("chat"), "code");
    expect(removeLibraryRule(db, "chat")).toBe(true);
    expect(removeLibraryRule(db, "chat")).toBe(false);
  });

  it("reports an unreadable row as quarantined and leaves it out of reads", async () => {
    const db = await temporaryDatabase();
    upsertLibraryRule(db, ruleCatalog, codeRule("good"), "code");
    db.insert(libraryRules).values({
      ruleId: "broken", kind: "code", document: { kind: "code", rule: { id: "broken" } } as never,
      revision: 1, payloadHash: "x", createdAt: 1, updatedAt: 1,
    }).run();

    expect(listLibraryRules(db).map(rule => rule.document.rule.id)).toEqual(["good"]);
    expect(quarantinedLibraryRules(db)).toEqual([expect.objectContaining({ ruleId: "broken", owner: "library" })]);
  });

  it("quarantines a row the schema accepts and the engine refuses, and refuses to export it", async () => {
    const db = await temporaryDatabase();
    upsertLibraryRule(db, ruleCatalog, codeRule("good"), "code");
    // Python has no `this`: the schema reads resolves.this as a shape, the engine refuses it for the language.
    const refused = {
      format: "rerail.policy/v1", id: "python-this", revision: 1, mode: "steer",
      clauses: [{
        id: "check",
        violation: { code: "TEST", anchor: "witness.span", message: "Test" },
        forbid: { ast: { language: "python", rule: { kind: "identifier", resolves: { this: "global" } } } },
      }],
    };
    db.insert(libraryRules).values({
      ruleId: "python-this", kind: "code", document: { kind: "code", rule: refused } as never,
      revision: 1, payloadHash: "x", createdAt: 1, updatedAt: 1,
    }).run();

    expect(listLibraryRules(db).map(rule => rule.document.rule.id)).toEqual(["good"]);
    expect(getLibraryRule(db, "python-this")).toBeUndefined();
    expect(quarantinedLibraryRules(db)).toEqual([{ ruleId: "python-this", kind: "code", owner: "library", reason: expect.stringMatching(/this/u) }]);
    expect(() => exportRulesDump(db)).toThrow(/cannot export 1 quarantined library rule/u);
  });
});

describe("copyRerailRuleToLibrary", () => {
  it("adds the approved copy wherever the original is present and switches the original off there", async () => {
    const db = await repositoryWithLintPack(["web", "api"]);
    const edited = codeRule("ignored", { message: "No print, ever" });

    const copied = copyRerailRuleToLibrary(db, ruleCatalog, REPOSITORY, "no-console", edited, { approve: true });
    expect(copied.rule).toMatchObject({ copiedFromRuleId: "no-console", document: { rule: { id: "no-console-copy" } } });
    expect(copied.folders).toEqual(["api", "web"]);
    expect(copyRerailRuleToLibrary(db, ruleCatalog, REPOSITORY, "no-console", edited, { approve: true }).rule.document.rule.id).toBe("no-console-copy-2");

    const rules = loadCheckoutRuleSet(db, ruleCatalog, checkout(CHECKOUT), RUNNING, []).rulesFor("web/a.ts");
    expect(rules.map(rule => [rule.document.rule.id, rule.source.kind, rule.on])).toEqual([
      ["no-console", "pack", false],
      ["no-console-copy", "library_rule", true],
      ["no-console-copy-2", "library_rule", true],
      ["no-var", "pack", false],
    ]);
  });

  it("leaves the original following the rule catalog", async () => {
    const db = await repositoryWithLintPack([""]);
    copyRerailRuleToLibrary(db, ruleCatalog, REPOSITORY, "no-console", codeRule("x", { message: "edited" }), { approve: true });

    const updated = testBundle({ noConsoleRevision: 2 });
    const original = loadCheckoutRuleSet(db, updated, checkout(CHECKOUT), RUNNING, []).rulesFor("a.ts")
      .find(rule => rule.document.rule.id === "no-console");
    expect(original).toMatchObject({ revision: 2, on: false, source: { kind: "pack" } });
  });

  it("lets a library rule win over a later bundled rule with its id", async () => {
    const db = await repositoryWithLintPack([]);
    writeLibraryRule(db, testBundle(), { kind: "code", rule: codeRule("shared", { mode: "log" }) }, { approve: true });
    const later = testBundle();
    const withShared = { ...later, documents: new Map([...later.documents, ["shared", { kind: "code" as const, rule: codeRule("shared") }]]) };
    addAttachment(db, { repositoryKey: REPOSITORY, folder: "", sourceKind: "rerail_rule", sourceId: "shared" });
    addAttachment(db, { repositoryKey: REPOSITORY, folder: "lib", sourceKind: "library_rule", sourceId: "shared" });

    const rules = loadCheckoutRuleSet(db, withShared, checkout(CHECKOUT), RUNNING, []);
    expect(rules.unavailable).toEqual([{ sourceKind: "rerail_rule", sourceId: "shared", folder: "", reason: "conflicts-with-library" }]);
    expect(rules.rulesFor("a.ts")).toEqual([]);
    expect(rules.rulesFor("lib/a.ts").map(rule => [rule.source.kind, rule.mode])).toEqual([["library_rule", "log"]]);
  });
});

describe("rules dump", () => {
  it("round-trips the library and repository configuration, keeping this machine's start state", async () => {
    const source = await repositoryWithLintPack(["web"]);
    upsertLibraryRule(source, ruleCatalog, codeRule("mine"), "code");
    addAttachment(source, { repositoryKey: REPOSITORY, folder: "api", sourceKind: "library_rule", sourceId: "mine" });
    setRuleSetting(source, { owner: { kind: "repository", repositoryKey: REPOSITORY }, folder: "web", targetKind: "pack", targetId: "lint", on: null, mode: "log" });
    setRuleSetting(source, { owner: { kind: "checkout", checkoutPath: CHECKOUT }, folder: "", targetKind: "pack", targetId: "lint", on: false, mode: null });
    const dump = JSON.parse(JSON.stringify(exportRulesDump(source))) as unknown;

    const target = await temporaryDatabase();
    addRepository(target, REPOSITORY);
    target.update(repositories).set({ started: true }).run();
    addAttachment(target, { repositoryKey: REPOSITORY, folder: "", sourceKind: "rerail_rule", sourceId: "loose" });
    // The builder's rules are on Block unless they name another mode.
    expect(importRulesDump(target, ruleCatalog, dump)).toEqual({ library: 1, repositories: 1, onBlock: ["mine"] });

    expect(exportRulesDump(target, new Date(0)).repositories).toEqual(exportRulesDump(source, new Date(0)).repositories);
    expect(getLibraryRule(target, "mine")?.document).toEqual(getLibraryRule(source, "mine")?.document);
    expect(target.select().from(repositories).get()?.started).toBe(true);
    expect(target.select().from(ruleSettings).all().map(row => row.ownerKind)).toEqual(["repository"]);
  });

  it("round-trips suppressions and config, writes neither when unset, and imports a setting from a dump that predates them", async () => {
    const source = await repositoryWithLintPack(["web"]);
    const owner = { kind: "repository" as const, repositoryKey: REPOSITORY };
    setRuleSetting(source, { owner, folder: "web", targetKind: "pack", targetId: "lint", on: null, mode: null, suppressions: "ignore" });
    setRuleSetting(source, { owner, folder: "", targetKind: "rule", targetId: "no-console", on: null, mode: "log", config: { word: "alert" } });
    const exported = exportRulesDump(source, new Date(0));
    expect(exported.repositories[0]!.settings).toEqual([
      { folder: "", targetKind: "rule", targetId: "no-console", on: null, mode: "log", config: { word: "alert" } },
      { folder: "web", targetKind: "pack", targetId: "lint", on: null, mode: null, suppressions: "ignore" },
    ]);

    const target = await temporaryDatabase();
    importRulesDump(target, ruleCatalog, JSON.parse(JSON.stringify(exported)) as unknown);
    expect(exportRulesDump(target, new Date(0)).repositories).toEqual(exported.repositories);

    const older = { ...exported, repositories: [{ ...exported.repositories[0]!, settings: [{ folder: "web", targetKind: "pack", targetId: "lint", on: false, mode: null }] }] };
    importRulesDump(target, ruleCatalog, JSON.parse(JSON.stringify(older)) as unknown);
    expect(target.select({ suppressions: ruleSettings.suppressions, config: ruleSettings.config, on: ruleSettings.on }).from(ruleSettings).all())
      .toEqual([{ suppressions: null, config: null, on: false }]);
  });

  it("keeps a library rule that a later release bundles editable, and round-trips it through a dump", async () => {
    const source = await temporaryDatabase();
    writeLibraryRule(source, testBundle(), { kind: "code", rule: codeRule("shared", { mode: "log" }) });
    const later = testBundle();
    const withShared = { ...later, documents: new Map([...later.documents, ["shared", { kind: "code" as const, rule: codeRule("shared") }]]) };
    expect(upsertLibraryRule(source, withShared, codeRule("shared", { mode: "steer" }), "code").document.rule.mode).toBe("steer");
    const dump = JSON.parse(JSON.stringify(exportRulesDump(source))) as unknown;

    expect(importRulesDump(source, withShared, dump)).toEqual({ library: 1, repositories: 0, onBlock: [] });
    const target = await temporaryDatabase();
    expect(importRulesDump(target, withShared, dump)).toEqual({ library: 1, repositories: 0, onBlock: [] });
    expect(getLibraryRule(target, "shared")?.document.rule.mode).toBe("steer");
  });

  it("imports a repository listed under one of this machine's aliases into the repository that holds the alias", async () => {
    const source = await repositoryWithLintPack(["web"]);
    const dump = JSON.parse(JSON.stringify(exportRulesDump(source))) as unknown;
    const target = await temporaryDatabase();
    addRepository(target, "remote:github.com/acme/app-renamed");
    target.update(repositories).set({ aliases: [REPOSITORY] }).run();

    expect(importRulesDump(target, ruleCatalog, dump)).toEqual({ library: 0, repositories: 1, onBlock: [] });
    expect(target.select({ key: repositories.repositoryKey, aliases: repositories.aliases }).from(repositories).all())
      .toEqual([{ key: "remote:github.com/acme/app-renamed", aliases: [REPOSITORY] }]);
    expect(target.select({ key: ruleAttachments.repositoryKey, folder: ruleAttachments.folder }).from(ruleAttachments).all())
      .toEqual([{ key: "remote:github.com/acme/app-renamed", folder: "web" }]);
  });

  it("names what the dump puts on Block, library rules and repository settings, each once", async () => {
    const source = await repositoryWithLintPack(["web"]);
    upsertLibraryRule(source, ruleCatalog, codeRule("mine", { mode: "block" }), "code");
    upsertLibraryRule(source, ruleCatalog, codeRule("calm", { mode: "steer" }), "code");
    const owner = { kind: "repository" as const, repositoryKey: REPOSITORY };
    setRuleSetting(source, { owner, folder: "web", targetKind: "pack", targetId: "lint", on: null, mode: "block" });
    setRuleSetting(source, { owner, folder: "", targetKind: "rule", targetId: "mine", on: null, mode: "block" });
    const dump = JSON.parse(JSON.stringify(exportRulesDump(source))) as unknown;

    const report = importRulesDump(await temporaryDatabase(), ruleCatalog, dump);

    expect([...report.onBlock].sort()).toEqual(["mine", "pack lint"]);
  });

  it("imports nothing when a code rule does not compile", async () => {
    const db = await temporaryDatabase();
    const broken = { ...codeRule("broken"), clauses: [{ ...codeRule("broken").clauses[0], forbid: { regex: { pattern: "(", source: "file.content" } } }] };
    const dump = {
      version: 1,
      exportedAt: "now",
      library: [
        { kind: "code", rule: codeRule("fine"), revision: 1, copiedFromRuleId: null, createdAt: 1, updatedAt: 1 },
        { kind: "code", rule: broken, revision: 1, copiedFromRuleId: null, createdAt: 1, updatedAt: 1 },
      ],
      repositories: [],
    };
    expect(() => importRulesDump(db, ruleCatalog, dump)).toThrow();
    expect(listLibraryRules(db)).toEqual([]);
  });

  it("rejects a dump of another version", async () => {
    const db = await temporaryDatabase();
    expect(() => importRulesDump(db, ruleCatalog, { version: 5, exportedAt: "now", rules: [] })).toThrow(/v5 rules dump can't be imported/u);
  });
});
