import { existsSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { FIXTURE_RULES, loadScoredRules } from "./fixtures.js";
import { goldenFailures } from "./golden-gate.js";
import { goldenRules, loadGoldenManifest } from "./golden.js";
import { evaluateRule, type RuleResult } from "./runner.js";

describe.skipIf(!existsSync(FIXTURE_RULES))("golden gate", () => {
  const rules = loadScoredRules();
  const evaluate = async (names: readonly string[]): Promise<RuleResult[]> => {
    const results: RuleResult[] = [];
    for (const name of names) results.push(await evaluateRule(rules.find(rule => rule.name === name)!, 1));
    return results;
  };

  it("holds for every golden rule", async () => {
    expect(goldenFailures(await evaluate(goldenRules()))).toEqual([]);
  }, 120_000);

  it("names each rule that misses its conformance.json entry", async () => {
    const manifest = loadGoldenManifest();
    const results = await evaluate(["no-debugger", "quotemark"]);
    const debuggerEntry = manifest.rules["no-debugger"]!;
    const quotemarkEntry = manifest.rules.quotemark!;
    const drifted = {
      ...manifest,
      rules: {
        "no-debugger": { ...debuggerEntry, truePositives: debuggerEntry.truePositives + 1 },
        quotemark: { ...quotemarkEntry, verdict: "PASS" as const, outOfScope: ["jsx-double/test.js.lint"] },
      },
    };
    expect(goldenFailures(results, drifted, ["no-debugger", "quotemark"])).toEqual([
      `no-debugger: TP ${debuggerEntry.truePositives}, golden expects ${debuggerEntry.truePositives + 1}`,
      "quotemark: effective PASS (repaired fixture), golden expects PASS",
      "quotemark: out-of-scope fixture files none, golden expects jsx-double/test.js.lint",
    ]);
  });
});
