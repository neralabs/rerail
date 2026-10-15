import { describe, expect, it } from "vitest";
import type { RerailDatabase } from "../../../src/engine/storage/database.js";
import type { BundledCatalog, EffectiveRule, RuleSetting } from "../../../src/engine/catalog/repository-model.js";
import { codeRuleSchema } from "../../../src/engine/code-rule/code-rule-schema.js";
import {
  addAttachment,
  listRepositoryRules,
  loadCheckoutRuleSet,
  removeAttachment,
  ruleConfigurationSignature,
  setRuleSetting,
  setUpRepository,
} from "../../../src/engine/catalog/rule-configuration.js";
import { removeLibraryRule, writeLibraryRule } from "../../../src/engine/catalog/rule-library.js";
import {
  addRepository,
  checkout,
  OTHER_REPOSITORY,
  codeRule,
  REPOSITORY,
  RUNNING,
  temporaryDatabase,
  testBundle,
} from "../../support/rule-configuration-builders.js";

const WORKTREE = "/work/app-feature";
const OTHER_CHECKOUT = "/work/app";
const bundle = testBundle();

type SettingValue = Omit<RuleSetting, "owner" | "folder" | "targetKind" | "targetId">;

function repositorySetting(folder: string, targetKind: "rule" | "pack", targetId: string, value: SettingValue): RuleSetting {
  return { owner: { kind: "repository", repositoryKey: REPOSITORY }, folder, targetKind, targetId, ...value };
}

function checkoutSetting(checkoutPath: string, folder: string, targetKind: "rule" | "pack", targetId: string, value: SettingValue): RuleSetting {
  return { owner: { kind: "checkout", checkoutPath }, folder, targetKind, targetId, ...value };
}

function ruleAt(db: RerailDatabase, checkoutPath: string, path: string, id: string, parents: readonly string[] = []): EffectiveRule | undefined {
  return loadCheckoutRuleSet(db, bundle, checkout(checkoutPath), RUNNING, parents).rulesFor(path).find(rule => rule.document.rule.id === id);
}

/** The proposal §5 worked example: the lint pack in `web/`, pack Log for the repository, `no-console` off in `web/legacy/`, Steer in this worktree. */
async function workedExample(): Promise<RerailDatabase> {
  const db = await temporaryDatabase();
  addRepository(db, REPOSITORY);
  setUpRepository(db, REPOSITORY, {
    packs: [{ packId: "lint", folder: "web/" }],
    settings: [
      repositorySetting("", "pack", "lint", { on: null, mode: "log" }),
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
    expect(here).toMatchObject({ on: true, mode: "steer", attachmentFolder: "web", source: { kind: "pack", id: "lint", packId: "lint" } });
    expect(here?.decidedBy).toEqual({
      on: { by: "pack-start", packId: "lint" },
      mode: { by: "checkout", checkoutPath: WORKTREE, folder: "", target: "rule" },
      suppressions: { by: "pack-start", packId: "lint" },
      config: {},
    });
    expect(here).toMatchObject({ suppressions: "honour", config: null, configError: null });
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
    setRuleSetting(db, checkoutSetting(WORKTREE, "", "pack", "lint", { on: false, mode: null }));

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

    expect(ruleAt(db, OTHER_CHECKOUT, "web/a.ts", "no-console")).toMatchObject({ source: { kind: "pack", packId: "lint" }, mode: "log" });
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

describe("suppressions", () => {
  it("start from the pack's honourSuppressionComments, else honour", async () => {
    const db = await workedExample();
    addAttachment(db, { repositoryKey: REPOSITORY, folder: "", sourceKind: "rerail_rule", sourceId: "loose" });
    const quiet = testBundle({ honourSuppressionComments: false });
    const at = (catalog: BundledCatalog, id: string) =>
      loadCheckoutRuleSet(db, catalog, checkout(OTHER_CHECKOUT), RUNNING, []).rulesFor("web/a.ts").find(rule => rule.document.rule.id === id);

    expect(at(bundle, "no-console")).toMatchObject({ suppressions: "honour", decidedBy: { suppressions: { by: "pack-start", packId: "lint" } } });
    expect(at(quiet, "no-console")).toMatchObject({ suppressions: "ignore", decidedBy: { suppressions: { by: "pack-start", packId: "lint" } } });
    expect(at(quiet, "loose")).toMatchObject({ suppressions: "honour", decidedBy: { suppressions: { by: "rule-default" } } });
  });

  it("follow the setting precedence: checkout over repository, deepest folder, rule over pack", async () => {
    const db = await workedExample();
    setRuleSetting(db, repositorySetting("", "pack", "lint", { on: null, mode: "log", suppressions: "ignore" }));
    expect(ruleAt(db, OTHER_CHECKOUT, "web/a.ts", "no-console")).toMatchObject({
      suppressions: "ignore", decidedBy: { suppressions: { by: "repository", folder: "", target: "pack" } },
    });

    setRuleSetting(db, repositorySetting("web/src", "pack", "lint", { on: null, mode: null, suppressions: "honour" }));
    expect(ruleAt(db, OTHER_CHECKOUT, "web/src/a.ts", "no-console")?.suppressions).toBe("honour");
    expect(ruleAt(db, OTHER_CHECKOUT, "web/a.ts", "no-console")?.suppressions).toBe("ignore");

    setRuleSetting(db, repositorySetting("", "rule", "no-var", { on: null, mode: null, suppressions: "honour" }));
    expect(ruleAt(db, OTHER_CHECKOUT, "web/a.ts", "no-var")).toMatchObject({
      suppressions: "honour", decidedBy: { suppressions: { by: "repository", folder: "", target: "rule" } },
    });

    setRuleSetting(db, checkoutSetting(WORKTREE, "", "pack", "lint", { on: null, mode: null, suppressions: "ignore" }));
    expect(ruleAt(db, WORKTREE, "web/src/a.ts", "no-console")).toMatchObject({
      suppressions: "ignore", decidedBy: { suppressions: { by: "checkout", checkoutPath: WORKTREE, folder: "", target: "pack" } },
    });
    expect(ruleAt(db, OTHER_CHECKOUT, "web/src/a.ts", "no-console")?.suppressions).toBe("honour");
  });

  it("is a setting of its own: a row may set only suppressions, and clearing it removes the row", async () => {
    const db = await workedExample();
    setRuleSetting(db, repositorySetting("web/legacy", "rule", "no-console", { on: false, mode: null, suppressions: "ignore" }));
    expect(ruleAt(db, OTHER_CHECKOUT, "web/legacy/a.ts", "no-console")).toMatchObject({ on: false, suppressions: "ignore" });
    setRuleSetting(db, repositorySetting("web/legacy", "rule", "no-console", { on: null, mode: null, suppressions: "ignore" }));
    expect(ruleAt(db, OTHER_CHECKOUT, "web/legacy/a.ts", "no-console")).toMatchObject({ on: true, suppressions: "ignore" });
    setRuleSetting(db, repositorySetting("web/legacy", "rule", "no-console", { on: null, mode: null, suppressions: null }));
    expect(ruleAt(db, OTHER_CHECKOUT, "web/legacy/a.ts", "no-console")?.decidedBy.suppressions).toEqual({ by: "pack-start", packId: "lint" });
  });
});

/** A By Rerail rule with two options: the word it forbids before `(` and its message. */
const configurableSource = {
  format: "rerail.policy/v1",
  id: "configurable",
  revision: 1,
  mode: "block",
  clauses: [
    { id: "options", define: { options: { word: { type: "string", default: "print" }, message: { type: "string", default: "No print" } } } },
    {
      id: "configurable:check",
      violation: { anchor: "witness.span", code: "FORBIDDEN_WORD", message: { $option: "message" } },
      forbid: { regex: { pattern: { $option: "word", template: "\\b{}\\(" }, source: "file.content" } },
    },
  ],
};
const configurableBundle: BundledCatalog = {
  ...bundle,
  documents: new Map([...bundle.documents, ["configurable", { kind: "code", rule: codeRuleSchema.parse(configurableSource) }]]),
  sources: new Map([...bundle.sources, ["configurable", configurableSource]]),
};

describe("config", () => {
  function configurableAt(db: RerailDatabase, checkoutPath: string, path = "a.ts"): EffectiveRule | undefined {
    return loadCheckoutRuleSet(db, configurableBundle, checkout(checkoutPath), RUNNING, []).rulesFor(path).find(rule => rule.document.rule.id === "configurable");
  }
  /** The forbidding clause of the effective document, with its option references resolved. */
  const forbidden = (rule: EffectiveRule | undefined) =>
    rule?.document.kind === "code" ? rule.document.rule.clauses.find(clause => clause.id === "configurable:check") : undefined;

  async function configurableExample(): Promise<RerailDatabase> {
    const db = await temporaryDatabase();
    addRepository(db, REPOSITORY);
    setUpRepository(db, REPOSITORY, { packs: [], rules: [{ sourceKind: "rerail_rule", sourceId: "configurable", folder: "" }] }, 10);
    return db;
  }

  it("leaves the document as written when no setting sets an option", async () => {
    const db = await configurableExample();
    expect(configurableAt(db, OTHER_CHECKOUT)).toMatchObject({ config: null, configError: null, decidedBy: { config: {} } });
    expect(forbidden(configurableAt(db, OTHER_CHECKOUT))).toMatchObject({ forbid: { regex: { pattern: "\\bprint\\(" } }, violation: { message: "No print" } });
  });

  it("merges options per option, the checkout's before the repository's, and resolves the document with them", async () => {
    const db = await configurableExample();
    setRuleSetting(db, repositorySetting("", "rule", "configurable", { on: null, mode: null, config: { word: "alert", message: "No alert" } }));
    setRuleSetting(db, checkoutSetting(WORKTREE, "", "rule", "configurable", { on: null, mode: "steer", config: { word: "debug" } }));

    const here = configurableAt(db, WORKTREE);
    expect(here).toMatchObject({ mode: "steer", config: { word: "debug", message: "No alert" }, configError: null });
    expect(here?.decidedBy.config).toEqual({
      word: { by: "checkout", checkoutPath: WORKTREE, folder: "", target: "rule" },
      message: { by: "repository", folder: "", target: "rule" },
    });
    expect(forbidden(here)).toMatchObject({ forbid: { regex: { pattern: "\\bdebug\\(" } }, violation: { message: "No alert" } });
    expect(here?.document.rule.mode).toBe("steer");
    expect(forbidden(configurableAt(db, OTHER_CHECKOUT))).toMatchObject({ forbid: { regex: { pattern: "\\balert\\(" } }, violation: { message: "No alert" } });
  });

  it("reports a stored config the document no longer declares instead of applying part of it", async () => {
    const db = await configurableExample();
    setRuleSetting(db, repositorySetting("", "rule", "configurable", { on: null, mode: null, config: { wrod: "alert" } }));
    const rule = configurableAt(db, OTHER_CHECKOUT);
    expect(rule?.config).toEqual({ wrod: "alert" });
    expect(rule?.configError).toMatch(/`wrod` is not an option this document declares/u);
    expect(forbidden(rule)).toMatchObject({ forbid: { regex: { pattern: "\\bprint\\(" } }, violation: { message: "No print" } });
  });

  it("takes no config from a folder setting, and gives none to a library rule", async () => {
    const db = await configurableExample();
    setRuleSetting(db, repositorySetting("web", "rule", "configurable", { on: null, mode: null, config: { word: "alert" } }));
    expect(configurableAt(db, OTHER_CHECKOUT, "web/a.ts")?.config).toBeNull();

    setRuleSetting(db, repositorySetting("", "rule", "mine", { on: null, mode: null, config: { word: "alert" } }));
    writeLibraryRule(db, configurableBundle, { kind: "code", rule: codeRule("mine") });
    addAttachment(db, { repositoryKey: REPOSITORY, folder: "", sourceKind: "library_rule", sourceId: "mine" });
    const library = loadCheckoutRuleSet(db, configurableBundle, checkout(OTHER_CHECKOUT), RUNNING, []).rulesFor("a.ts").find(rule => rule.document.rule.id === "mine");
    expect(library).toMatchObject({ source: { kind: "library_rule" }, config: null, configError: null });
  });
});

describe("rule configuration writes", () => {
  it("normalises folders and removes a setting when both fields are cleared", async () => {
    const db = await workedExample();
    expect(removeAttachment(db, { repositoryKey: REPOSITORY, folder: "./web/", sourceKind: "pack", sourceId: "lint" })).toBe(true);
    expect(ruleAt(db, OTHER_CHECKOUT, "web/a.ts", "no-console")).toBeUndefined();

    addAttachment(db, { repositoryKey: REPOSITORY, folder: "", sourceKind: "pack", sourceId: "lint" });
    setRuleSetting(db, repositorySetting("", "pack", "lint", { on: null, mode: null }));
    expect(ruleAt(db, OTHER_CHECKOUT, "web/a.ts", "no-console")?.decidedBy.mode).toEqual({ by: "pack-start", packId: "lint" });
  });

  it("rejects folders outside the repository", async () => {
    const db = await workedExample();
    expect(() => addAttachment(db, { repositoryKey: REPOSITORY, folder: "../x", sourceKind: "pack", sourceId: "lint" })).toThrow(/inside the repository/u);
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

    setRuleSetting(db, { owner: { kind: "repository", repositoryKey: OTHER_REPOSITORY }, folder: "", targetKind: "pack", targetId: "lint", on: false, mode: null });
    setRuleSetting(db, checkoutSetting("/elsewhere", "", "pack", "lint", { on: false, mode: null }));
    expect(ruleConfigurationSignature(db, bundle, REPOSITORY, [WORKTREE])).toBe(before);

    setRuleSetting(db, checkoutSetting(WORKTREE, "web", "rule", "no-var", { on: true, mode: null }));
    const afterCheckout = ruleConfigurationSignature(db, bundle, REPOSITORY, [WORKTREE]);
    expect(afterCheckout).not.toBe(before);

    setRuleSetting(db, repositorySetting("web", "rule", "no-var", { on: false, mode: null }));
    expect(ruleConfigurationSignature(db, bundle, REPOSITORY, [WORKTREE])).not.toBe(afterCheckout);
    expect(ruleConfigurationSignature(db, testBundle({ noConsoleRevision: 2 }), REPOSITORY, [WORKTREE]))
      .not.toBe(ruleConfigurationSignature(db, bundle, REPOSITORY, [WORKTREE]));
  });

  it("changes when only suppressions or config change", async () => {
    const db = await workedExample();
    const before = ruleConfigurationSignature(db, bundle, REPOSITORY, []);
    setRuleSetting(db, repositorySetting("", "pack", "lint", { on: null, mode: "log", suppressions: "ignore" }));
    const ignored = ruleConfigurationSignature(db, bundle, REPOSITORY, []);
    expect(ignored).not.toBe(before);
    setRuleSetting(db, repositorySetting("", "rule", "no-console", { on: null, mode: null, config: { word: "alert" } }));
    const configured = ruleConfigurationSignature(db, bundle, REPOSITORY, []);
    expect(configured).not.toBe(ignored);
    setRuleSetting(db, repositorySetting("", "rule", "no-console", { on: null, mode: null, config: { word: "debug" } }));
    expect(ruleConfigurationSignature(db, bundle, REPOSITORY, [])).not.toBe(configured);
  });

  it("reports a pack rule a library rule shadows, and changes when that library rule goes", async () => {
    const db = await workedExample();
    writeLibraryRule(db, testBundle(), { kind: "code", rule: codeRule("shared") });
    const pack = bundle.packs.get("lint")!;
    const withShared: BundledCatalog = {
      ...bundle,
      documents: new Map([...bundle.documents, ["shared", { kind: "code", rule: codeRule("shared") }]]),
      packs: new Map([["lint", { ...pack, rules: [...pack.rules, { ...pack.rules[0]!, id: "shared" }] }]]),
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
