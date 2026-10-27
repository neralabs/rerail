import assert from "node:assert/strict";
import { test } from "vitest";

import { conformanceChecks, type PackConformance } from "../../conformance/languages/checks/conformance.js";
import type { RuleRow } from "../../conformance/languages/report.js";

type Counts = Partial<Pick<RuleRow, "tp" | "fp" | "fn" | "engineFailures" | "examples" | "reviewed" | "cleanSamples" | "silentCleanSamples" | "ownUnits" | "silentOwnUnits" | "renamed" | "syntaxErrorFiles">>;

function row(rule: string, counts: Counts = {}, source = "fixtures"): RuleRow {
  return {
    rule, candidate: true, examples: false, upstream: { tool: "ruff", rule: "X1" }, capabilities: [], source, tp: 2, fp: 0, fn: 0,
    precision: 1, recall: 1, reviewed: 0, reviewedFindings: [], cleanSamples: 0, silentCleanSamples: 0,
    // By default the rule has one own upstream unit it and upstream are silent on.
    ownUnits: 1, silentOwnUnits: 1, upstreamFindings: 2, renamed: null,
    engineFailures: 0, contextFailures: 0, partialFiles: 0, syntaxErrorFiles: { count: 0, files: [] }, divergences: [], falsePositives: [], falseNegatives: [], ...counts,
  };
}

function conformance(rows: RuleRow[], extra: Partial<PackConformance> = {}): PackConformance {
  return { rows: new Map(rows.map((entry) => [entry.rule, entry])), sampleRows: new Map(), stale: new Map(), ruleErrors: new Map(), errors: [], ...extra };
}

const failing = (pack: PackConformance, id: string) => conformanceChecks(pack, id).filter((check) => !check.ok).map((check) => check.id);

test("a rule passes when the pack's check would pass it as shipped", () => {
  assert.deepEqual(failing(conformance([row("a")]), "a"), []);
  assert.deepEqual(failing(conformance([row("a", { tp: 0, examples: true })]), "a"), []);
});

test("an unexplained finding on the pack's fixtures or probes fails it, naming the first, as the check fails a shipped rule", () => {
  const withFp = { ...row("a", { fp: 1 }), falsePositives: [{ path: "rerail/probes/A1.py", line: 5, column: 23 }] };
  const all = conformance([withFp, row("b", { fn: 2 }), row("c", { tp: 0 }), row("d", { engineFailures: 1 })]);
  const checks = conformanceChecks(all, "a");
  assert.deepEqual(checks.filter((check) => !check.ok).map((check) => check.id), ["no-unexplained-false-positives"]);
  assert.match(checks.find((check) => !check.ok)!.detail, /1 unexplained FP on the pack's fixtures, first rerail\/probes\/A1\.py:5:23/u);
  assert.deepEqual(failing(all, "b"), ["no-unexplained-misses"]);
  assert.deepEqual(failing(all, "c"), ["catches-something"]);
  assert.deepEqual(failing(all, "d"), ["engine-reads-fixtures"]);
});

test("a stale divergence, an error naming the rule or a missing row fails it", () => {
  assert.deepEqual(failing(conformance([row("a")], { stale: new Map([["a", ["x.py:3 (upstream-wrong)"]]]) }), "a"), ["divergences-current"]);
  const missingTree = conformance([row("a")], { ruleErrors: new Map([["a", ["a (samples): no samples tree at .rerail/pack-data/samples/python/ruff/0123"]]]) });
  assert.deepEqual(failing(missingTree, "a"), ["scored"]);
  assert.deepEqual(failing(conformance([]), "missing"), ["fixtures-row"]);
});

test("a missed label, a finding on a clean sample or an engine failure fails it; a reviewed report or a sample read in part does not; a rule no sample judges is not judged there", () => {
  const judged = (counts: Counts) => failing(conformance([row("a")], { sampleRows: new Map([["a", row("a", counts, "samples")]]) }), "a");
  assert.deepEqual(judged({}), []);
  assert.deepEqual(judged({ reviewed: 3 }), []);
  assert.deepEqual(judged({ fn: 1 }), ["samples-catch-labelled"]);
  assert.deepEqual(judged({ fp: 1 }), ["samples-silent-on-clean"]);
  assert.deepEqual(judged({ engineFailures: 1 }), ["engine-reads-samples"]);
  assert.deepEqual(judged({ syntaxErrorFiles: { count: 1, files: [{ path: "s/a.py", line: 2, column: 1 }] } }), []);
  const ids = conformanceChecks(conformance([row("a")]), "a").map((check) => check.id);
  assert.ok(!ids.some((id) => id.startsWith("samples-") || id === "engine-reads-samples"));
});

test("stays-silent: an own upstream unit upstream and the rule are silent on, or a silent clean sample, meets it, and the detail names each", () => {
  const staysSilent = (own: number, silentOwn: number, samples: number | undefined) =>
    conformanceChecks(
      conformance([row("a", { ownUnits: own, silentOwnUnits: silentOwn })], samples === undefined ? {} : { sampleRows: new Map([["a", row("a", { cleanSamples: samples, silentCleanSamples: samples }, "samples")]]) }),
      "a",
    ).find((check) => check.id === "stays-silent")!;
  assert.deepEqual(staysSilent(4, 3, 2), { id: "stays-silent", ok: true, detail: "silent where it should be: 3 of 4 own upstream unit(s), 2 clean sample(s)" });
  assert.equal(staysSilent(1, 1, undefined).ok, true);
  assert.equal(staysSilent(0, 0, 1).ok, true);
  // Own units it is not silent on prove nothing, and neither does a run with no clean sample.
  assert.equal(staysSilent(3, 0, 0).ok, false);
  assert.match(staysSilent(0, 0, undefined).detail, /^no own upstream unit upstream reports nothing in \(0 own unit\(s\)\) and no clean sample/u);
});

test("holds-under-rename: an unexplained FP or FN on a rename variant fails it; without variants the samples are the held-out evidence", () => {
  const renamed = (fp: number, fn: number): RuleRow["renamed"] => ({ units: 4, tp: 3, fp, fn, falsePositives: fp === 0 ? [] : [{ path: "renamed/a/001.js", line: 2, column: 5 }], falseNegatives: [] });
  const check = (value: RuleRow["renamed"]) => conformanceChecks(conformance([row("a", { renamed: value })]), "a").find((entry) => entry.id === "holds-under-rename")!;
  assert.deepEqual(check(null), { id: "holds-under-rename", ok: true, detail: "n/a: samples are the held-out evidence" });
  assert.deepEqual(check(renamed(0, 0)), { id: "holds-under-rename", ok: true, detail: "agrees with upstream on its 4 rename variant unit(s) (3 true positive(s))" });
  assert.equal(check(renamed(1, 0)).ok, false);
  assert.match(check(renamed(1, 0)).detail, /^1 unexplained FP, first renamed\/a\/001\.js:2:5 and 0 unexplained FN on rename variants/u);
  assert.equal(check(renamed(0, 2)).ok, false);
});
