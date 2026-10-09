import { describe, expect, it } from "vitest";
import type { RerailDatabase } from "../src/database.js";
import type { BundledCatalog, EffectiveRule, RuleSetting } from "../src/repository-model.js";
import {
  addAttachment,
  listRepositoryRules,
  loadCheckoutRuleSet,
  removeAttachment,
  ruleConfigurationSignature,
  setRuleSetting,
  setUpRepository,
} from "../src/rule-configuration.js";
import { removeLibraryRule, writeLibraryRule } from "../src/rule-library.js";
import {
  addRepository,
  checkout,
  OTHER_REPOSITORY,
  codeRule,
  REPOSITORY,
  RUNNING,
  temporaryDatabase,
  testBundle,
} from "./support/rule-configuration-fixture.js";

const WORKTREE = "/work/app-feature";
const OTHER_CHECKOUT = "/work/app";
const bundle = testBundle();

function repositorySetting(folder: string, targetKind: "rule" | "pack", targetId: string, value: Pick<RuleSetting, "on" | "mode">): RuleSetting {
  return { owner: { kind: "repository", repositoryKey: REPOSITORY }, folder, targetKind, targetId, ...value };
}

function checkoutSetting(checkoutPath: string, folder: string, targetKind: "rule" | "pack", targetId: string, value: Pick<RuleSetting, "on" | "mode">): RuleSetting {
  return { owner: { kind: "checkout", checkoutPath }, folder, targetKind, targetId, ...value };
}

function ruleAt(db: RerailDatabase, checkoutPath: string, path: string, id: string, parents: readonly string[] = []): EffectiveRule | undefined {
  return loadCheckoutRuleSet(db, bundle, checkout(checkoutPath), RUNNING, parents).rulesFor(path).find(rule => rule.document.rule.id === id);
}

/** The proposal §5 worked example: TSLint in `web/`, pack Log for the repository, `no-console` off in `web/legacy/`, Steer in this worktree. */
async function workedExample(): Promise<RerailDatabase> {
  const db = await temporaryDatabase();
  addRepository(db, REPOSITORY);
  setUpRepository(db, REPOSITORY, {
    packs: [{ packId: "tslint", folder: "web/" }],
    settings: [
      repositorySetting("", "pack", "tslint", { on: null, mode: "log" }),
      repositorySetting("web/legacy", "rule", "no-console", { on: false, mode: null }),
    ],
  }, 10);
  setRuleSetting(db, checkoutSetting(WORKTREE, "", "rule", "no-console", { on: null, mode: "steer" }));
  return db;
}

describe("loadCheckoutRuleSet precedence", () => {
  it("resolves the worked example per checkout and folder", async () => {
    const db = await workedExample();

    const here = ruleAt(db, WORKTREE, "web/src/app.ts", "no-console");
    expect(here).toMatchObject({ on: true, mode: "steer", attachmentFolder: "web", source: { kind: "pack", id: "tslint", packId: "tslint" } });
    expect(here?.decidedBy).toEqual({
      on: { by: "pack-start", packId: "tslint" },
      mode: { by: "checkout", checkoutPath: WORKTREE, folder: "", target: "rule" },
    });
    expect(here?.document.rule.mode).toBe("steer");

    const elsewhere = ruleAt(db, OTHER_CHECKOUT, "web/src/app.ts", "no-console");
    expect(elsewhere).toMatchObject({ on: true, mode: "log", decidedBy: { mode: { by: "repository", folder: "", target: "pack" } } });

    for (const path of [WORKTREE, OTHER_CHECKOUT]) {
      expect(ruleAt(db, path, "web/legacy/old.ts", "no-console")).toMatchObject({
        on: false,
        decidedBy: { on: { by: "repository", folder: "web/legacy", target: "rule" } },
      });
    }
  });

  it("applies an owner's setting before a deeper folder of a later owner", async () => {
    const db = await workedExample();
    setRuleSetting(db, repositorySetting("web/src", "rule", "no-console", { on: null, mode: "block" }));

    expect(ruleAt(db, WORKTREE, "web/src/app.ts", "no-console")?.mode).toBe("steer");
    expect(ruleAt(db, OTHER_CHECKOUT, "web/src/app.ts", "no-console")?.mode).toBe("block");
  });

  it("applies an owner's pack setting before a later owner's rule setting", async () => {
    const db = await workedExample();
    setRuleSetting(db, repositorySetting("", "rule", "no-var", { on: true, mode: null }));
    setRuleSetting(db, checkoutSetting(WORKTREE, "", "pack", "tslint", { on: false, mode: null }));

    expect(ruleAt(db, WORKTREE, "web/a.ts", "no-var")?.on).toBe(false);
    expect(ruleAt(db, OTHER_CHECKOUT, "web/a.ts", "no-var")?.on).toBe(true);
  });

  it("lets a parent checkout's setting reach a nested checkout, nearest first", async () => {
    const db = await workedExample();
    const nested = `${WORKTREE}/vendor/app`;
    expect(ruleAt(db, nested, "web/a.ts", "no-console", [WORKTREE])?.decidedBy.mode).toEqual({
      by: "checkout", checkoutPath: WORKTREE, folder: "", target: "rule",
    });

    setRuleSetting(db, checkoutSetting(nested, "", "rule", "no-console", { on: null, mode: "block" }));
    expect(ruleAt(db, nested, "web/a.ts", "no-console", [WORKTREE])?.mode).toBe("block");
  });

  it("treats a rule in an added pack as the pack's even when it is also added on its own", async () => {
    const db = await workedExample();
    addAttachment(db, { repositoryKey: REPOSITORY, folder: "", sourceKind: "rerail_rule", sourceId: "no-console" });

    expect(ruleAt(db, OTHER_CHECKOUT, "web/a.ts", "no-console")).toMatchObject({ source: { kind: "pack", packId: "tslint" }, mode: "log" });
    expect(ruleAt(db, OTHER_CHECKOUT, "api/a.ts", "no-console")).toMatchObject({
      source: { kind: "rerail_rule", id: "no-console", packId: null },
      attachmentFolder: "",
      on: true,
      mode: "block",
      decidedBy: { on: { by: "rule-default" }, mode: { by: "rule-default" } },
    });
  });

  it("ignores a setting for a rule that is not added", async () => {
    const db = await workedExample();
    setRuleSetting(db, repositorySetting("", "rule", "loose", { on: true, mode: "block" }));
    expect(ruleAt(db, OTHER_CHECKOUT, "web/a.ts", "loose")).toBeUndefined();
  });

  it("does not let folder web contain web2", async () => {
    const db = await workedExample();
    const rules = loadCheckoutRuleSet(db, bundle, checkout(OTHER_CHECKOUT), RUNNING, []);
    expect(rules.rulesFor("web2/x.ts")).toEqual([]);
    expect(rules.rulesFor("web").map(rule => rule.document.rule.id)).toEqual(["no-console", "no-var"]);
  });

  it("reports a missing pack, rule or library rule as unavailable without throwing", async () => {
    const db = await workedExample();
    addAttachment(db, { repositoryKey: REPOSITORY, folder: "", sourceKind: "pack", sourceId: "gone" });
    addAttachment(db, { repositoryKey: REPOSITORY, folder: "api", sourceKind: "rerail_rule", sourceId: "nope" });
    addAttachment(db, { repositoryKey: REPOSITORY, folder: "", sourceKind: "library_rule", sourceId: "absent" });

    const rules = loadCheckoutRuleSet(db, bundle, checkout(OTHER_CHECKOUT), RUNNING, []);
    expect(rules.unavailable).toEqual(expect.arrayContaining([
      { sourceKind: "pack", sourceId: "gone", folder: "", reason: "missing" },
      { sourceKind: "rerail_rule", sourceId: "nope", folder: "api", reason: "missing" },
      { sourceKind: "library_rule", sourceId: "absent", folder: "", reason: "missing" },
    ]));
    expect(rules.unavailable).toHaveLength(3);
    expect(rules.rulesFor("api/a.ts")).toEqual([]);
  });

  it("returns conversation rules for the starting folder", async () => {
    const db = await workedExample();
    addAttachment(db, { repositoryKey: REPOSITORY, folder: "api", sourceKind: "rerail_rule", sourceId: "talk" });
    setRuleSetting(db, repositorySetting("api", "rule", "talk", { on: null, mode: "log" }));
    const rules = loadCheckoutRuleSet(db, bundle, checkout(OTHER_CHECKOUT), RUNNING, []);

    expect(rules.conversationRules("")).toEqual([]);
    expect(rules.conversationRules("api/routes").map(rule => [rule.document.rule.id, rule.mode])).toEqual([["talk", "log"]]);
    expect(rules.rulesFor("api/routes/a.ts").some(rule => rule.document.kind === "conversation")).toBe(false);
  });

  it("is empty unless the checkout is started and its repository set up", async () => {
    const db = await workedExample();
    for (const status of [{ ...RUNNING, started: false }, { ...RUNNING, setUp: false }]) {
      const rules = loadCheckoutRuleSet(db, bundle, checkout(OTHER_CHECKOUT), status, []);
      expect(rules.rulesFor("web/a.ts")).toEqual([]);
      expect(rules.conversationRules("web")).toEqual([]);
    }
  });
});

describe("rule configuration writes", () => {
  it("normalises folders and removes a setting when both fields are cleared", async () => {
    const db = await workedExample();
    expect(removeAttachment(db, { repositoryKey: REPOSITORY, folder: "./web/", sourceKind: "pack", sourceId: "tslint" })).toBe(true);
    expect(ruleAt(db, OTHER_CHECKOUT, "web/a.ts", "no-console")).toBeUndefined();

    addAttachment(db, { repositoryKey: REPOSITORY, folder: "", sourceKind: "pack", sourceId: "tslint" });
    setRuleSetting(db, repositorySetting("", "pack", "tslint", { on: null, mode: null }));
    expect(ruleAt(db, OTHER_CHECKOUT, "web/a.ts", "no-console")?.decidedBy.mode).toEqual({ by: "pack-start", packId: "tslint" });
  });

  it("rejects folders outside the repository", async () => {
    const db = await workedExample();
    expect(() => addAttachment(db, { repositoryKey: REPOSITORY, folder: "../x", sourceKind: "pack", sourceId: "tslint" })).toThrow(/inside the repository/u);
  });

  it("sets up a repository with nothing added and refuses an unknown one", async () => {
    const db = await temporaryDatabase();
    addRepository(db, REPOSITORY);
    setUpRepository(db, REPOSITORY, { packs: [] }, 42);
    expect(listRepositoryRules(db, bundle, REPOSITORY).rules).toEqual([]);
    expect(() => setUpRepository(db, OTHER_REPOSITORY, { packs: [] })).toThrow(/Unknown repository/u);
  });
});

describe("ruleConfigurationSignature", () => {
  it("changes with a setting of this repository or checkout, not of another repository", async () => {
    const db = await workedExample();
    addRepository(db, OTHER_REPOSITORY);
    const before = ruleConfigurationSignature(db, bundle, REPOSITORY, [WORKTREE]);

    setRuleSetting(db, { owner: { kind: "repository", repositoryKey: OTHER_REPOSITORY }, folder: "", targetKind: "pack", targetId: "tslint", on: false, mode: null });
    setRuleSetting(db, checkoutSetting("/elsewhere", "", "pack", "tslint", { on: false, mode: null }));
    expect(ruleConfigurationSignature(db, bundle, REPOSITORY, [WORKTREE])).toBe(before);

    setRuleSetting(db, checkoutSetting(WORKTREE, "web", "rule", "no-var", { on: true, mode: null }));
    const afterCheckout = ruleConfigurationSignature(db, bundle, REPOSITORY, [WORKTREE]);
    expect(afterCheckout).not.toBe(before);

    setRuleSetting(db, repositorySetting("web", "rule", "no-var", { on: false, mode: null }));
    expect(ruleConfigurationSignature(db, bundle, REPOSITORY, [WORKTREE])).not.toBe(afterCheckout);
    expect(ruleConfigurationSignature(db, testBundle({ noConsoleRevision: 2 }), REPOSITORY, [WORKTREE]))
      .not.toBe(ruleConfigurationSignature(db, bundle, REPOSITORY, [WORKTREE]));
  });

  it("reports a pack rule a library rule shadows, and changes when that library rule goes", async () => {
    const db = await workedExample();
    writeLibraryRule(db, testBundle(), { kind: "code", rule: codeRule("shared") });
    const pack = bundle.packs.get("tslint")!;
    const withShared: BundledCatalog = {
      ...bundle,
      documents: new Map([...bundle.documents, ["shared", { kind: "code", rule: codeRule("shared") }]]),
      packs: new Map([["tslint", { ...pack, rules: [...pack.rules, { ...pack.rules[0]!, id: "shared" }] }]]),
    };

    const rules = loadCheckoutRuleSet(db, withShared, checkout(OTHER_CHECKOUT), RUNNING, []);
    expect(rules.unavailable).toEqual([{ sourceKind: "rerail_rule", sourceId: "shared", folder: "web", reason: "conflicts-with-library" }]);
    expect(rules.rulesFor("web/a.ts").map(rule => rule.document.rule.id)).toEqual(["no-console", "no-var"]);

    const before = ruleConfigurationSignature(db, withShared, REPOSITORY, []);
    removeLibraryRule(db, "shared");
    expect(ruleConfigurationSignature(db, withShared, REPOSITORY, [])).not.toBe(before);
    expect(loadCheckoutRuleSet(db, withShared, checkout(OTHER_CHECKOUT), RUNNING, []).rulesFor("web/a.ts").map(rule => rule.document.rule.id))
      .toEqual(["no-console", "no-var", "shared"]);
  });
});

describe("listRepositoryRules", () => {
  it("lists each rule at its attachment folder with the source of each value", async () => {
    const db = await workedExample();
    const repositoryView = listRepositoryRules(db, bundle, REPOSITORY);
    expect(repositoryView.rules.map(rule => [rule.document.rule.id, rule.attachmentFolder, rule.on, rule.mode])).toEqual([
      ["no-console", "web", true, "log"],
      ["no-var", "web", false, "log"],
    ]);
    const worktreeView = listRepositoryRules(db, bundle, REPOSITORY, { checkoutPath: WORKTREE });
    expect(worktreeView.rules.find(rule => rule.document.rule.id === "no-console")?.decidedBy.mode).toMatchObject({ by: "checkout" });
  });
});
