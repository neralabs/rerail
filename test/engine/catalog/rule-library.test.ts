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

const bundle = testBundle();
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
    expect(() => upsertLibraryRule(db, bundle, codeRule("no-console"), "code")).toThrow(/By Rerail rule id/u);

    upsertLibraryRule(db, bundle, codeRule("mine"), "code");
    expect(() => upsertLibraryRule(db, bundle, conversation("mine"), "conversation")).toThrow(/exists with kind code/u);
    expect(getLibraryRule(db, "mine")?.document.kind).toBe("code");
  });

  it("bumps a conversation rule's revision on each write and removes everything that points at a rule", async () => {
    const db = await repositoryWithLintPack([""]);
    upsertLibraryRule(db, bundle, conversation("chat"), "conversation");
    expect(upsertLibraryRule(db, bundle, conversation("chat"), "conversation").revision).toBe(2);
    addAttachment(db, { repositoryKey: REPOSITORY, folder: "api", sourceKind: "library_rule", sourceId: "chat" });
    setRuleSetting(db, { owner: { kind: "repository", repositoryKey: REPOSITORY }, folder: "api", targetKind: "rule", targetId: "chat", on: false, mode: null });

    expect(removeLibraryRule(db, "chat")).toBe(true);
    expect(db.select().from(ruleAttachments).all().map(row => row.sourceKind)).toEqual(["pack"]);
    expect(db.select().from(ruleSettings).all()).toEqual([]);
    upsertLibraryRule(db, bundle, codeRule("chat"), "code");
    expect(removeLibraryRule(db, "chat")).toBe(true);
    expect(removeLibraryRule(db, "chat")).toBe(false);
  });

  it("reports an unreadable row as quarantined and leaves it out of reads", async () => {
    const db = await temporaryDatabase();
    upsertLibraryRule(db, bundle, codeRule("good"), "code");
    db.insert(libraryRules).values({
      ruleId: "broken", kind: "code", document: { kind: "code", rule: { id: "broken" } } as never,
      revision: 1, payloadHash: "x", createdAt: 1, updatedAt: 1,
    }).run();

    expect(listLibraryRules(db).map(rule => rule.document.rule.id)).toEqual(["good"]);
    expect(quarantinedLibraryRules(db)).toEqual([expect.objectContaining({ ruleId: "broken", owner: "library" })]);
  });
});

describe("copyRerailRuleToLibrary", () => {
  it("adds the copy wherever the original is present and switches the original off there", async () => {
    const db = await repositoryWithLintPack(["web", "api"]);
    const edited = codeRule("ignored", { message: "No print, ever" });

    const copied = copyRerailRuleToLibrary(db, bundle, REPOSITORY, "no-console", edited);
    expect(copied.rule).toMatchObject({ copiedFromRuleId: "no-console", document: { rule: { id: "no-console-copy" } } });
    expect(copied.folders).toEqual(["api", "web"]);
    expect(copyRerailRuleToLibrary(db, bundle, REPOSITORY, "no-console", edited).rule.document.rule.id).toBe("no-console-copy-2");

    const rules = loadCheckoutRuleSet(db, bundle, checkout(CHECKOUT), RUNNING, []).rulesFor("web/a.ts");
    expect(rules.map(rule => [rule.document.rule.id, rule.source.kind, rule.on])).toEqual([
      ["no-console", "pack", false],
      ["no-console-copy", "library_rule", true],
      ["no-console-copy-2", "library_rule", true],
      ["no-var", "pack", false],
    ]);
  });

  it("leaves the original following the bundle", async () => {
    const db = await repositoryWithLintPack([""]);
    copyRerailRuleToLibrary(db, bundle, REPOSITORY, "no-console", codeRule("x", { message: "edited" }));

    const updated = testBundle({ noConsoleRevision: 2 });
    const original = loadCheckoutRuleSet(db, updated, checkout(CHECKOUT), RUNNING, []).rulesFor("a.ts")
      .find(rule => rule.document.rule.id === "no-console");
    expect(original).toMatchObject({ revision: 2, on: false, source: { kind: "pack" } });
  });

  it("lets a library rule win over a later bundled rule with its id", async () => {
    const db = await repositoryWithLintPack([]);
    writeLibraryRule(db, testBundle(), { kind: "code", rule: codeRule("shared", { mode: "log" }) });
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
    upsertLibraryRule(source, bundle, codeRule("mine"), "code");
    addAttachment(source, { repositoryKey: REPOSITORY, folder: "api", sourceKind: "library_rule", sourceId: "mine" });
    setRuleSetting(source, { owner: { kind: "repository", repositoryKey: REPOSITORY }, folder: "web", targetKind: "pack", targetId: "lint", on: null, mode: "log" });
    setRuleSetting(source, { owner: { kind: "checkout", checkoutPath: CHECKOUT }, folder: "", targetKind: "pack", targetId: "lint", on: false, mode: null });
    const dump = JSON.parse(JSON.stringify(exportRulesDump(source))) as unknown;

    const target = await temporaryDatabase();
    addRepository(target, REPOSITORY);
    target.update(repositories).set({ started: true }).run();
    addAttachment(target, { repositoryKey: REPOSITORY, folder: "", sourceKind: "rerail_rule", sourceId: "loose" });
    expect(importRulesDump(target, bundle, dump)).toEqual({ library: 1, repositories: 1 });

    expect(exportRulesDump(target, new Date(0)).repositories).toEqual(exportRulesDump(source, new Date(0)).repositories);
    expect(getLibraryRule(target, "mine")?.document).toEqual(getLibraryRule(source, "mine")?.document);
    expect(target.select().from(repositories).get()?.started).toBe(true);
    expect(target.select().from(ruleSettings).all().map(row => row.ownerKind)).toEqual(["repository"]);
  });

  it("keeps a library rule that a later release bundles editable, and round-trips it through a dump", async () => {
    const source = await temporaryDatabase();
    writeLibraryRule(source, testBundle(), { kind: "code", rule: codeRule("shared", { mode: "log" }) });
    const later = testBundle();
    const withShared = { ...later, documents: new Map([...later.documents, ["shared", { kind: "code" as const, rule: codeRule("shared") }]]) };
    expect(upsertLibraryRule(source, withShared, codeRule("shared", { mode: "steer" }), "code").document.rule.mode).toBe("steer");
    const dump = JSON.parse(JSON.stringify(exportRulesDump(source))) as unknown;

    expect(importRulesDump(source, withShared, dump)).toEqual({ library: 1, repositories: 0 });
    const target = await temporaryDatabase();
    expect(importRulesDump(target, withShared, dump)).toEqual({ library: 1, repositories: 0 });
    expect(getLibraryRule(target, "shared")?.document.rule.mode).toBe("steer");
  });

  it("imports a repository listed under one of this machine's aliases into the repository that holds the alias", async () => {
    const source = await repositoryWithLintPack(["web"]);
    const dump = JSON.parse(JSON.stringify(exportRulesDump(source))) as unknown;
    const target = await temporaryDatabase();
    addRepository(target, "remote:github.com/acme/app-renamed");
    target.update(repositories).set({ aliases: [REPOSITORY] }).run();

    expect(importRulesDump(target, bundle, dump)).toEqual({ library: 0, repositories: 1 });
    expect(target.select({ key: repositories.repositoryKey, aliases: repositories.aliases }).from(repositories).all())
      .toEqual([{ key: "remote:github.com/acme/app-renamed", aliases: [REPOSITORY] }]);
    expect(target.select({ key: ruleAttachments.repositoryKey, folder: ruleAttachments.folder }).from(ruleAttachments).all())
      .toEqual([{ key: "remote:github.com/acme/app-renamed", folder: "web" }]);
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
    expect(() => importRulesDump(db, bundle, dump)).toThrow();
    expect(listLibraryRules(db)).toEqual([]);
  });

  it("rejects a dump of another version", async () => {
    const db = await temporaryDatabase();
    expect(() => importRulesDump(db, bundle, { version: 5, exportedAt: "now", rules: [] })).toThrow(/v5 rules dump can't be imported/u);
  });
});
