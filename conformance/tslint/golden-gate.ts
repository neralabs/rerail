/**
 * The golden gate: every golden rule (`golden.ts`) must meet its golden.json entry on every full run and on
 * `just rerail-conformance golden`.
 */
import { goldenRules, loadGoldenManifest, membershipProblems, type GoldenEntry } from "./golden.js";
import { effectiveVerdict, type RuleResult } from "./runner.js";

/** The entry a run supports for `result`, or why it supports none (not a PASS, or a PASS that is not clean). */
export function observedEntry(result: RuleResult): { entry: GoldenEntry } | { problem: string } {
  const effective = effectiveVerdict(result);
  if (effective !== "PASS" && effective !== "PASS (repaired fixture)") return { problem: `effective ${effective}` };
  const score = effective === "PASS" ? result : result.repaired!;
  return {
    entry: {
      verdict: effective,
      truePositives: score.truePositives,
      ...(result.outOfScope.length === 0 ? {} : { outOfScope: result.outOfScope }),
    },
  };
}

/** Per-rule regressions of a run against the golden manifest; empty when every golden rule meets its entry. */
export function goldenFailures(results: readonly RuleResult[], manifest = loadGoldenManifest(), rules = goldenRules()): string[] {
  const failures = membershipProblems(manifest, rules);
  const byRule = new Map(results.map(result => [result.rule, result]));
  for (const rule of rules) {
    const expected = manifest.rules[rule];
    const result = byRule.get(rule);
    if (expected === undefined) continue;
    if (result === undefined) {
      failures.push(`${rule}: not evaluated`);
      continue;
    }
    const score = expected.verdict === "PASS" ? result : result.repaired;
    const counts = score === undefined ? "no repaired score" : `TP ${score.truePositives} FP ${score.falsePositives} FN ${score.falseNegatives} engine failures ${score.engineFailures}`;
    const observed = observedEntry(result);
    if ("problem" in observed) {
      failures.push(`${rule}: ${observed.problem} (${counts}), golden expects ${expected.verdict} with TP ${expected.truePositives}`);
      continue;
    }
    if (observed.entry.verdict !== expected.verdict) failures.push(`${rule}: effective ${observed.entry.verdict}, golden expects ${expected.verdict}`);
    else if (observed.entry.truePositives !== expected.truePositives) failures.push(`${rule}: TP ${observed.entry.truePositives}, golden expects ${expected.truePositives}`);
    const outOfScope = (list: readonly string[] | undefined): string => (list ?? []).join(", ") || "none";
    if (outOfScope(observed.entry.outOfScope) !== outOfScope(expected.outOfScope)) {
      failures.push(`${rule}: out-of-scope fixture files ${outOfScope(observed.entry.outOfScope)}, golden expects ${outOfScope(expected.outOfScope)}`);
    }
  }
  return failures;
}
