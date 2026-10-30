import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { eq } from "drizzle-orm";
import { describe, expect, it } from "vitest";
import type { RerailDatabase } from "../../../src/engine/storage/database.js";
import type { RuleCatalog } from "../../../src/engine/catalog/repository-model.js";
import { approveRule, pendingRules, RuleApprovalError } from "../../../src/engine/catalog/rule-approval.js";
import { installedPacksPrefix, registryInstalledPacks } from "../../../src/engine/catalog/rule-catalog.js";
import { listRepositoryRules, loadCheckoutRuleSet, setUpRepository } from "../../../src/engine/catalog/rule-configuration.js";
import { copyRerailRuleToLibrary, getLibraryRule, removeLibraryRule, upsertLibraryRule, writeLibraryRule } from "../../../src/engine/catalog/rule-library.js";
import { ruleApprovals } from "../../../src/engine/storage/schema.js";
import {
  addRepository,
  checkout,
  codeRule,
  REPOSITORY,
  RUNNING,
  temporaryDatabase,
  temporaryDirectory,
  testBundle,
} from "../../support/rule-configuration-builders.js";

const WORKTREE = "/work/app";
const shipped = testBundle();
/** The same catalog with nothing Nera shipped: its pack stands for one added from a folder or tarball. */
const local: RuleCatalog = { ...testBundle(), shipped: new Set() };

function running(db: RerailDatabase, ruleCatalog: RuleCatalog, id: string) {
  return loadCheckoutRuleSet(db, ruleCatalog, checkout(WORKTREE), RUNNING, []).rulesFor("src/app.ts").find(rule => rule.document.rule.id === id);
}

async function repositoryWith(rules: { packs?: string[]; library?: string[] }): Promise<RerailDatabase> {
  const db = await temporaryDatabase();
  addRepository(db, REPOSITORY);
  setUpRepository(db, REPOSITORY, {
    packs: (rules.packs ?? []).map(packId => ({ packId, folder: "" })),
    rules: (rules.library ?? []).map(sourceId => ({ sourceKind: "library_rule" as const, sourceId, folder: "" })),
  });
  return db;
}

function contentHashOf(db: RerailDatabase, id: string): string {
  return getLibraryRule(db, id)!.contentHash;
}

describe("rules created by you wait for a person's approval", () => {
  it("runs a new rule nowhere until its content is approved, and reports it as waiting", async () => {
    const db = await repositoryWith({ library: ["mine"] });
    writeLibraryRule(db, shipped, { kind: "code", rule: codeRule("mine") });

    expect(running(db, shipped, "mine")).toBeUndefined();
    const view = listRepositoryRules(db, shipped, REPOSITORY);
    expect(view.unavailable).toContainEqual({ sourceKind: "library_rule", sourceId: "mine", folder: "", reason: "awaiting-approval" });
    expect(view.awaitingApproval).toBe(1);
    expect(pendingRules(db, shipped).map(pending => [pending.owner, pending.document.rule.id, pending.approved])).toEqual([["library", "mine", undefined]]);

    approveRule(db, shipped, "library", "mine", contentHashOf(db, "mine"));
    expect(running(db, shipped, "mine")).toMatchObject({ on: true, approval: { state: "approved" } });
    expect(listRepositoryRules(db, shipped, REPOSITORY).awaitingApproval).toBe(0);
    expect(pendingRules(db, shipped)).toEqual([]);
  });

  it("keeps the approved content running while an edit waits, then runs the edit once approved", async () => {
    const db = await repositoryWith({ library: ["mine"] });
    upsertLibraryRule(db, shipped, codeRule("mine", { message: "approved words" }), "code", { approve: true });
    upsertLibraryRule(db, shipped, codeRule("mine", { message: "edited words" }), "code");

    const before = running(db, shipped, "mine");
    expect(before).toMatchObject({ on: true, approval: { state: "pending" } });
    expect(JSON.stringify(before!.document.rule)).toContain("approved words");
    expect(JSON.stringify(before!.document.rule)).not.toContain("edited words");
    const [pending] = pendingRules(db, shipped);
    expect(JSON.stringify(pending!.approved!.document.rule)).toContain("approved words");

    approveRule(db, shipped, "library", "mine", pending!.contentHash);
    expect(JSON.stringify(running(db, shipped, "mine")!.document.rule)).toContain("edited words");
  });

  it("needs no new approval for a change of mode or revision alone, which are settings", async () => {
    const db = await repositoryWith({ library: ["mine"] });
    upsertLibraryRule(db, shipped, codeRule("mine", { mode: "block" }), "code", { approve: true });
    upsertLibraryRule(db, shipped, codeRule("mine", { mode: "log", revision: 2 }), "code");
    expect(running(db, shipped, "mine")).toMatchObject({ mode: "log", revision: 2, approval: { state: "approved" } });
  });

  it("approves a person's own write as it is saved", async () => {
    const db = await repositoryWith({ library: ["mine"] });
    upsertLibraryRule(db, shipped, codeRule("mine"), "code", { approve: true });
    expect(running(db, shipped, "mine")).toMatchObject({ approval: { state: "approved" } });
  });

  it("refuses to approve content other than what the person read", async () => {
    const db = await repositoryWith({ library: ["mine"] });
    upsertLibraryRule(db, shipped, codeRule("mine", { message: "first" }), "code");
    const read = contentHashOf(db, "mine");
    upsertLibraryRule(db, shipped, codeRule("mine", { message: "swapped in after the person read it" }), "code");

    expect(() => approveRule(db, shipped, "library", "mine", read)).toThrow(RuleApprovalError);
    expect(running(db, shipped, "mine")).toBeUndefined();
  });

  it("does not run content whose approval row was changed behind the hash", async () => {
    const db = await repositoryWith({ library: ["mine"] });
    upsertLibraryRule(db, shipped, codeRule("mine", { message: "approved words" }), "code", { approve: true });
    upsertLibraryRule(db, shipped, codeRule("mine", { message: "never approved" }), "code");
    // A row claiming the waiting content is approved, without a hash that matches what it stores.
    db.update(ruleApprovals)
      .set({ document: { kind: "code", rule: codeRule("mine", { message: "never approved" }) } })
      .where(eq(ruleApprovals.ruleId, "mine")).run();

    expect(running(db, shipped, "mine")).toBeUndefined();
    expect(listRepositoryRules(db, shipped, REPOSITORY).unavailable)
      .toContainEqual({ sourceKind: "library_rule", sourceId: "mine", folder: "", reason: "awaiting-approval" });
  });

  it("forgets the approval with the rule, so a rule written again under its id waits", async () => {
    const db = await repositoryWith({ library: ["mine"] });
    upsertLibraryRule(db, shipped, codeRule("mine"), "code", { approve: true });
    removeLibraryRule(db, "mine");
    upsertLibraryRule(db, shipped, codeRule("mine"), "code");
    expect(pendingRules(db, shipped).map(pending => pending.document.rule.id)).toEqual(["mine"]);
  });
});

describe("an edited copy of a By Rerail rule", () => {
  it("leaves the original running until the copy is approved, then switches it off where the copy is added", async () => {
    const db = await repositoryWith({ packs: ["lint"] });
    const copied = copyRerailRuleToLibrary(db, shipped, REPOSITORY, "no-console", codeRule("no-console", { message: "edited" }));
    const copyId = copied.rule.document.rule.id;

    expect(running(db, shipped, "no-console")).toMatchObject({ on: true });
    expect(running(db, shipped, copyId)).toBeUndefined();

    approveRule(db, shipped, "library", copyId, copied.rule.contentHash);
    expect(running(db, shipped, "no-console")).toMatchObject({ on: false });
    expect(running(db, shipped, copyId)).toMatchObject({ on: true, approval: { state: "approved" } });
  });

  it("does not let a library rule that waits for approval shadow the By Rerail rule of its id", async () => {
    const db = await repositoryWith({ packs: ["lint"] });
    writeLibraryRule(db, shipped, { kind: "code", rule: codeRule("no-console", { message: "imported" }) }, { shadowBundled: true });
    expect(running(db, shipped, "no-console")).toMatchObject({ on: true, source: { kind: "pack" }, approval: { state: "shipped" } });
  });
});

describe("pack rules", () => {
  it("runs the rules of a pack Nera shipped as shipped", async () => {
    const db = await repositoryWith({ packs: ["lint"] });
    expect(running(db, shipped, "no-console")).toMatchObject({ on: true, approval: { state: "shipped", approvedAt: null } });
    expect(pendingRules(db, shipped)).toEqual([]);
    expect(() => approveRule(db, shipped, "pack", "no-console", "sha256:any")).toThrow(/needs no approval/u);
  });

  it("holds the rules of a pack added from a folder or tarball until a person approves them", async () => {
    const db = await repositoryWith({ packs: ["lint"] });
    expect(running(db, local, "no-console")).toBeUndefined();
    expect(listRepositoryRules(db, local, REPOSITORY).unavailable)
      .toContainEqual({ sourceKind: "rerail_rule", sourceId: "no-console", folder: "", reason: "awaiting-approval" });

    const pending = pendingRules(db, local).find(entry => entry.document.rule.id === "no-console")!;
    expect(pending).toMatchObject({ owner: "pack", packId: "lint", approved: undefined });
    approveRule(db, local, "pack", "no-console", pending.contentHash);
    expect(running(db, local, "no-console")).toMatchObject({ on: true, approval: { state: "approved" } });

    // An update of the local pack waits too, and the content approved before keeps running.
    const updated: RuleCatalog = {
      ...local,
      documents: new Map([...local.documents, ["no-console", { kind: "code", rule: codeRule("no-console", { message: "updated pack" }) }]]),
    };
    const kept = running(db, updated, "no-console");
    expect(kept).toMatchObject({ on: true, approval: { state: "pending" } });
    expect(JSON.stringify(kept!.document.rule)).not.toContain("updated pack");
  });

  it("counts a pack as Nera's only when npm recorded it from the registry", async () => {
    const home = await temporaryDirectory();
    mkdirSync(installedPacksPrefix(home), { recursive: true });
    writeFileSync(join(installedPacksPrefix(home), "package.json"), JSON.stringify({
      dependencies: {
        "@neralabs/rerail-pack-registry": "0.1.1",
        "@neralabs/rerail-pack-ranged": "^0.2.0",
        "@neralabs/rerail-pack-folder": "file:../my-pack",
        "@neralabs/rerail-pack-tarball": "file:/tmp/rerail-pack-tarball-0.1.0.tgz",
        "@neralabs/rerail-pack-git": "github:someone/pack",
        "left-pad": "1.0.0",
      },
    }));
    expect([...registryInstalledPacks(home)].sort()).toEqual(["ranged", "registry"]);
  });
});
