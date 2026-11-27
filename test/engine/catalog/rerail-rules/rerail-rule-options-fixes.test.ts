/**
 * A pack's rule written in `.rerail` that declares options, offers fixes and honours its tool's suppression comments
 * (decision 0720), through the product's path: the fixture pack `fixtures/packs/rr-curly` ships the translation of
 * ESLint's curly. Its options are checked as a JSON rule's are and reach the run's `option` facts from a setting, a
 * pack's `config` or the repository's ESLint config; a finding's fix is the one a JSON rule's violation carries and a
 * steer shows; and its `suppress` lines make it a port of ESLint's curly, which check 2 lets it carry.
 */
import { mkdirSync, mkdtempSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { rerailProofFailures } from "../../../../conformance/rerail-pack-rules.js";
import { readRepositoryLintConfig } from "../../../../src/engine/catalog/lint-config/repository-lint-config.js";
import { declaredPackRuleConfigs } from "../../../../src/engine/catalog/packs-file/pack-config.js";
import { describeRerailRules, type RerailDescription } from "../../../../src/engine/catalog/rerail-rules/rerail-rule.js";
import { configureBundledCodeRule } from "../../../../src/engine/catalog/rule-config.js";
import type { RuleConfig } from "../../../../src/engine/catalog/repository-model.js";
import { readRuleCatalog } from "../../../../src/engine/catalog/rule-catalog.js";
import type { RerailRuleEntry } from "../../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { renderSteer } from "../../../../src/engine/code-rule/code-rule.js";
import { runRerailPolicies, rrRunOptions } from "../../../../src/engine/rr/rr-runner.js";
import { rrViolations } from "../../../../src/engine/rr/rr-violations.js";
import { fixturesPath } from "../../../support/oracles.js";
import { removeFolderSync } from "../../../support/platform/remove-folder.js";

const PACK = fixturesPath("packs", "rr-curly");
const RULE = "eslint-curly";
const RULE_FILE = join(PACK, "rules", `${RULE}.rerail`);
/** Starting the runner's child reads core's library with no kept store: seconds on a loaded machine. */
const CHILD_TIMEOUT = 120_000;

const temporary: string[] = [];
afterEach(() => {
  for (const directory of temporary.splice(0)) removeFolderSync(directory);
});

/** A folder holding `files`, removed after the test. */
function folder(files: Readonly<Record<string, string>>): string {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-rr-curly-")));
  temporary.push(root);
  for (const [path, text] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), text);
  }
  return root;
}

/** The rule as the checkout catalog places it, everywhere in steer, with the repository's option values `config`. */
function entry(config: RuleConfig | null): RerailRuleEntry {
  return {
    id: RULE, path: RULE_FILE, placement: [{ folder: "", mode: "steer" }], scope: { include: ["**/*"], exclude: [], text: [] },
    language: "typescript", files: [], config,
    provenance: { ruleId: RULE, revision: 3, payloadHash: "hash", source: { kind: "pack", id: "rr-curly", packId: "rr-curly" }, repositoryKey: "repository", checkoutPath: "/repo", placement: [] },
  };
}

/** `if (ready) { start(); }` on line 1, `if (done) stop();` on line 2. */
const SOURCE = "if (ready) { start(); }\nif (done) stop();\n";

describe("a pack's .rerail rule with options, fixes and suppression comments", { timeout: CHILD_TIMEOUT }, () => {
  it("is read with the options it declares and each violation's suppress and fix lines", () => {
    const catalog = readRuleCatalog([PACK]);
    expect(catalog.skipped).toEqual([]);
    const document = catalog.documents.get(RULE);
    expect(document?.kind).toBe("rerail");
    if (document?.kind !== "rerail") return;
    expect(Object.keys(document.rule.options)).toEqual(["option", "option2"]);
    expect(document.rule.options.option).toMatchObject({ type: "string", default: "all" });
    const suppress = [{ tool: "eslint", rules: ["curly"] }, { tool: "oxlint", rules: ["curly"] }, { tool: "biome", rules: ["lint/style/useBlockStatements"] }];
    expect(document.rule.violations).toEqual([
      { name: "MISSING_BRACES", suppress, fixes: ["add_braces"] },
      { name: "EXTRA_BRACES", suppress, fixes: ["drop_braces"] },
    ]);
  });

  it("takes option values its .options allows, from a setting, a pack's config or ESLint's config, and refuses others", () => {
    const catalog = readRuleCatalog([PACK]);
    expect(configureBundledCodeRule(catalog, RULE, { option: "multi", option2: "consistent" })).toMatchObject({ ok: true });
    expect(configureBundledCodeRule(catalog, RULE, { option: "sometimes" })).toEqual({ ok: false, reason: "config.option: is not one of the option's enum values" });
    expect(configureBundledCodeRule(catalog, RULE, { ceiling: 3 })).toEqual({ ok: false, reason: "config.ceiling: `ceiling` is not an option this rule declares" });
    const declared = { kind: "repository", path: "packs/rr-curly", config: { option: "multi" } } as const;
    expect(declaredPackRuleConfigs(catalog, declared, "rr-curly", ".rerail/packs.json")).toEqual([{ ruleId: RULE, config: { option: "multi" } }]);
    // ESLint's options for curly map onto the port's, through the names its `suppress eslint "curly"` line honours.
    const repository = folder({ ".eslintrc.json": JSON.stringify({ rules: { curly: ["error", "multi", "consistent"] } }), "src/a.ts": SOURCE });
    expect(readRepositoryLintConfig(repository, catalog, ["rr-curly"]).settings).toMatchObject([{ ruleId: RULE, config: { option: "multi", option2: "consistent" } }]);
  });

  it("runs with its options as option facts, and its findings carry the fix a steer shows", async () => {
    const repository = folder({ "src/a.ts": SOURCE });
    const run = async (config: RuleConfig | null) => {
      const outcome = await runRerailPolicies({ root: repository, policies: [RULE_FILE], options: rrRunOptions([entry(config)]) });
      if (outcome.status !== "completed") throw new Error(JSON.stringify(outcome));
      expect(outcome.failures).toEqual([]);
      return rrViolations(outcome.findings, [entry(config)], () => true);
    };
    // "all", the default: the unbraced body is reported, and the fix wraps it in braces.
    const all = await run(null);
    expect(all.map(({ code, path, span }) => [code, path, span?.startLine])).toEqual([["curly", "src/a.ts", 1]]);
    const body = SOURCE.indexOf("stop();");
    expect(all[0]?.fix).toEqual({
      span: { startByte: body, endByte: body + "stop();".length, startLine: 1, startColumn: "if (done) ".length, endLine: 1, endColumn: "if (done) stop();".length },
      text: "{stop();}",
    });
    expect(renderSteer(all)).toContain("Suggested fix: replace 2:11–2:18 with `{stop();}`");
    // "multi": braces only where they are needed, so the braced body is reported instead. Its fix removes braces
    // through the library's `.edit unbrace`, which is not run: the finding offers no fix.
    const multi = await run({ option: "multi" });
    expect(multi.map(({ code, message, span }) => [code, message, span?.startLine])).toEqual([["curly", "Unnecessary { after 'if' condition.", 0]]);
    expect(multi[0]?.fix).toBeUndefined();
  });

  it("may carry suppress lines as a port of ESLint's curly, and only those of the tools ESLint's family reads", () => {
    const [row] = describeRerailRules([RULE_FILE]);
    expect(row?.ok).toBe(true);
    const described = row as RerailDescription;
    expect(rerailProofFailures(RULE, described, { tool: "eslint", rule: "curly" })).toEqual([]);
    expect(rerailProofFailures(RULE, described, undefined)).toEqual([
      `${RULE}: a violation sets suppress, and the rule ports no tool: give it a port entry in conformance.json, or drop the suppress lines`,
    ]);
    expect(rerailProofFailures(RULE, described, { tool: "ruff", rule: "E501" })).toEqual(
      ["MISSING_BRACES", "EXTRA_BRACES"].flatMap(violation => ["eslint", "oxlint", "biome"].map(tool =>
        `${RULE}: ${violation} sets suppress ${tool}, whose comments are not ruff's; a port honours only the comments of the tool it ports`)),
    );
  });
});
