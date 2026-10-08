import { describe, expect, it } from "vitest";
import { exportRulesDump, importRulesDump } from "../src/rule-bootstrap.js";
import { addAttachment, loadCheckoutRuleSet, setRuleSetting, setUpRepository } from "../src/rule-configuration.js";
import {
  copyRerailRuleToLibrary,
  getLibraryRule,
  listLibraryRules,
  quarantinedLibraryRules,
  removeLibraryRule,
  upsertLibraryRule,
  writeLibraryRule,
} from "../src/rule-library.js";
import { libraryRules, repositories, ruleAttachments, ruleSettings } from "../src/schema.js";
import {
  addRepository,
  checkout,
  conversation,
  codeRule,
  REPOSITORY,
  RUNNING,
  temporaryDatabase,
  testBundle,
} from "./support/rule-configuration-fixture.js";

const bundle = testBundle();
const CHECKOUT = "/work/app";

async function repositoryWithTslint(folders: readonly string[]) {
  const db = await temporaryDatabase();
  addRepository(db, REPOSITORY);
  setUpRepository(db, REPOSITORY, { packs: folders.map(folder => ({ packId: "tslint", folder })) });
  return db;
}

describe("library rules", () => {
  it("rejects a By Rerail rule id and keeps one kind per id", async () => {
    const db = await temporaryDatabase();
    expect(() => upsertLibraryRule(db, bundle, codeRule("no-console"), "file")).toThrow(/By Rerail rule id/u);

    upsertLibraryRule(db, bundle, codeRule("mine"), "file");
    expect(() => upsertLibraryRule(db, bundle, conversation("mine"), "conversation")).toThrow(/exists with kind file/u);
    expect(getLibraryRule(db, "mine")?.document.kind).toBe("file");
  });

  it("bumps a conversation rule's revision on each write and removes everything that points at a rule", async () => {
    const db = await repositoryWithTslint([""]);
    upsertLibraryRule(db, bundle, conversation("chat"), "conversation");
    expect(upsertLibraryRule(db, bundle, conversation("chat"), "conversation").revision).toBe(2);
    addAttachment(db, { repositoryKey: REPOSITORY, folder: "api", sourceKind: "library_rule", sourceId: "chat" });
    setRuleSetting(db, { owner: { kind: "repository", repositoryKey: REPOSITORY }, folder: "api", targetKind: "rule", targetId: "chat", on: false, mode: null });

    expect(removeLibraryRule(db, "chat")).toBe(true);
    expect(db.select().from(ruleAttachments).all().map(row => row.sourceKind)).toEqual(["pack"]);
    expect(db.select().from(ruleSettings).all()).toEqual([]);
    upsertLibraryRule(db, bundle, codeRule("chat"), "file");
    expect(removeLibraryRule(db, "chat")).toBe(true);
    expect(removeLibraryRule(db, "chat")).toBe(false);
  });

  it("reports an unreadable row as quarantined and leaves it out of reads", async () => {
    const db = await temporaryDatabase();
    upsertLibraryRule(db, bundle, codeRule("good"), "file");
    db.insert(libraryRules).values({
      ruleId: "broken", kind: "file", document: { kind: "file", rule: { id: "broken" } } as never,
      revision: 1, payloadHash: "x", createdAt: 1, updatedAt: 1,
    }).run();

    expect(listLibraryRules(db).map(rule => rule.document.rule.id)).toEqual(["good"]);
    expect(quarantinedLibraryRules(db)).toEqual([expect.objectContaining({ ruleId: "broken", owner: "library" })]);
  });
});

describe("copyRerailRuleToLibrary", () => {
  it("adds the copy wherever the original is present and switches the original off there", async () => {
    const db = await repositoryWithTslint(["web", "api"]);
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
    const db = await repositoryWithTslint([""]);
    copyRerailRuleToLibrary(db, bundle, REPOSITORY, "no-console", codeRule("x", { message: "edited" }));

    const updated = testBundle({ noConsoleRevision: 2 });
    const original = loadCheckoutRuleSet(db, updated, checkout(CHECKOUT), RUNNING, []).rulesFor("a.ts")
      .find(rule => rule.document.rule.id === "no-console");
    expect(original).toMatchObject({ revision: 2, on: false, source: { kind: "pack" } });
  });

  it("lets a library rule win over a later bundled rule with its id", async () => {
    const db = await repositoryWithTslint([]);
    writeLibraryRule(db, testBundle(), { kind: "file", rule: codeRule("shared", { mode: "log" }) });
    const later = testBundle();
    const withShared = { ...later, documents: new Map([...later.documents, ["shared", { kind: "file" as const, rule: codeRule("shared") }]]) };
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
    const source = await repositoryWithTslint(["web"]);
    upsertLibraryRule(source, bundle, codeRule("mine"), "file");
    addAttachment(source, { repositoryKey: REPOSITORY, folder: "api", sourceKind: "library_rule", sourceId: "mine" });
    setRuleSetting(source, { owner: { kind: "repository", repositoryKey: REPOSITORY }, folder: "web", targetKind: "pack", targetId: "tslint", on: null, mode: "log" });
    setRuleSetting(source, { owner: { kind: "checkout", checkoutPath: CHECKOUT }, folder: "", targetKind: "pack", targetId: "tslint", on: false, mode: null });
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

  it("imports nothing when a file rule does not compile", async () => {
    const db = await temporaryDatabase();
    const broken = { ...codeRule("broken"), clauses: [{ ...codeRule("broken").clauses[0], forbid: { regex: { pattern: "(", source: "file.content" } } }] };
    const dump = {
      version: 1,
      exportedAt: "now",
      library: [
        { kind: "file", rule: codeRule("fine"), revision: 1, copiedFromRuleId: null, createdAt: 1, updatedAt: 1 },
        { kind: "file", rule: broken, revision: 1, copiedFromRuleId: null, createdAt: 1, updatedAt: 1 },
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
