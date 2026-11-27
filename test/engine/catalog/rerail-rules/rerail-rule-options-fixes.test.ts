/**
 * A pack's rule written in `.rerail` that reads options, offers fixes and names its tool's suppression comments
 * (decision 0720), through the product's path: the fixture pack `fixtures/packs/rr-curly` ships the translation of
 * ESLint's curly. The options it takes are those its rules read, and their values reach the run's `option` facts from
 * a setting, a pack's `config` or the repository's ESLint config; a finding's fix is the one a JSON rule's violation carries and a
 * steer shows; and its `suppress` lines make it a port of ESLint's curly, which check 2 lets it carry.
 */
import { mkdirSync, mkdtempSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { rerailSuppressionGaps } from "../../../../conformance/pack-validation.js";
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
  it("is read with the options its rules read and each violation's suppress and fix lines", () => {
    const catalog = readRuleCatalog([PACK]);
    expect(catalog.skipped).toEqual([]);
    const document = catalog.documents.get(RULE);
    expect(document?.kind).toBe("rerail");
    if (document?.kind !== "rerail") return;
    expect(document.rule.options).toEqual(["option", "option2"]);
    const suppress = [{ tool: "eslint", rules: ["curly"] }, { tool: "oxlint", rules: ["curly"] }, { tool: "biome", rules: ["lint/style/useBlockStatements"] }];
    expect(document.rule.violations).toEqual([
      { name: "MISSING_BRACES", suppress, fixes: ["add_braces"] },
      { name: "EXTRA_BRACES", suppress, fixes: ["drop_braces"] },
    ]);
  });

  it("takes values for the options it reads, from a setting, a pack's config or ESLint's config, and refuses any other name", () => {
    const catalog = readRuleCatalog([PACK]);
    expect(configureBundledCodeRule(catalog, RULE, { option: "multi", option2: "consistent" })).toMatchObject({ ok: true });
    expect(configureBundledCodeRule(catalog, RULE, { ceiling: 3 })).toEqual({ ok: false, reason: "config.ceiling: `ceiling` is not an option this rule reads" });
    expect(configureBundledCodeRule(catalog, RULE, { toString: true })).toEqual({ ok: false, reason: "config.toString: `toString` is not an option this rule reads" });
    // The rule states no allowed values, so a value it does not expect is accepted: the rule decides what it means.
    expect(configureBundledCodeRule(catalog, RULE, { option: "sometimes" })).toMatchObject({ ok: true });
    expect(configureBundledCodeRule(catalog, RULE, { option: Number.MAX_SAFE_INTEGER + 2 })).toEqual({ ok: false, reason: "config.option: must be text, a number, true, false or null, or a list of them" });
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
    // No option set, which the rule reads as "all": the unbraced body is reported, and the fix wraps it in braces.
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

describe("check 4 on a .rerail port's suppress lines", { timeout: CHILD_TIMEOUT }, () => {
  let read: RerailDescription | undefined;
  /** The engine's reading of curly, read once, by the first test that asks; a reading the engine refuses fails it. */
  const curly = (): RerailDescription => {
    if (read !== undefined) return read;
    const [row] = describeRerailRules([RULE_FILE]);
    if (row === undefined || !row.ok) throw new Error(`the engine does not read ${RULE_FILE}: ${row?.ok === false ? row.error : "no reading"}`);
    read = row;
    return row;
  };
  /** The rule curly reads as, with `violations` in place of its own. */
  const withViolations = (language: string, violations: RerailDescription["violations"]): RerailDescription => ({ ...curly(), language, violations });
  const line = (tool: string, ...rules: string[]) => ({ tool, rules });

  it("finds no gap in curly's eslint, oxlint and biome lines on each violation", () => {
    expect(rerailSuppressionGaps(curly(), "eslint", "curly")).toEqual({ unsuppressed: [], oxlintApart: [], biomeMissing: { clauses: [], rules: [] }, biomeAlone: [] });
  });

  it("names a violation without its tool's rule, and one with ESLint's comments without Oxlint's", () => {
    const described = withViolations("typescript", [
      { name: "A", suppress: [line("eslint", "curly")], fixes: [] },
      { name: "B", suppress: [line("eslint", "curly"), line("oxlint", "curly")], fixes: [] },
      { name: "C", suppress: [], fixes: [] },
    ]);
    expect(rerailSuppressionGaps(described, "eslint", "curly")).toMatchObject({ unsuppressed: ["C"], oxlintApart: ["A"] });
  });

  it("reads Ruff's and flake8's lines as noqa codes, and a PHP_CodeSniffer sniff's message codes as its sniff", () => {
    const ruff = withViolations("python", [
      { name: "X", suppress: [line("ruff", "E501")], fixes: [] },
      { name: "Y", suppress: [line("noqa", "E501")], fixes: [] },
      { name: "Z", suppress: [line("flake8", "W291")], fixes: [] },
    ]);
    expect(rerailSuppressionGaps(ruff, "ruff", "E501").unsuppressed).toEqual(["Z"]);
    const phpcs = withViolations("php", [{ name: "L", suppress: [line("phpcs", "Generic.Files.LineLength.TooLong")], fixes: [] }]);
    expect(rerailSuppressionGaps(phpcs, "phpcs", "Generic.Files.LineLength").unsuppressed).toEqual([]);
  });
});
