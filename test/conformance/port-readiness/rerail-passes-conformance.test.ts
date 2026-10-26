import assert from "node:assert/strict";
import { test } from "vitest";

import { passesConformanceChecks, type PackConformance } from "../../../conformance/port-readiness/checks/passes-conformance.js";
import type { RuleRow } from "../../../conformance/languages/report.js";
import { verdict } from "../../../conformance/port-readiness/types.js";

type Counts = Partial<Pick<RuleRow, "tp" | "fp" | "fn" | "engineFailures" | "examples" | "reviewed" | "cleanSamples" | "silentCleanSamples" | "syntaxErrorFiles">>;

function row(rule: string, counts: Counts = {}, source = "fixtures"): RuleRow {
  return {
    rule, candidate: true, examples: false, upstream: { tool: "ruff", rule: "X1" }, capabilities: [], source, tp: 2, fp: 0, fn: 0,
    precision: 1, recall: 1, reviewed: 0, reviewedFindings: [], cleanSamples: 0, silentCleanSamples: 0, engineFailures: 0, partialFiles: 0,
    syntaxErrorFiles: { count: 0, files: [] }, divergences: [], falsePositives: [], falseNegatives: [], ...counts,
  };
}

function conformance(rows: RuleRow[], extra: Partial<PackConformance> = {}): PackConformance {
  return { rows: new Map(rows.map((entry) => [entry.rule, entry])), sampleRows: new Map(), stale: new Map(), ruleErrors: new Map(), errors: [], ...extra };
}

/** The checks a rule fails; by default it has one own upstream case upstream and the rule are silent on. */
const failing = (pack: PackConformance, id: string, ownSilentCases = 1) =>
  passesConformanceChecks(pack, id, ownSilentCases).filter((check) => !check.ok).map((check) => check.id);

test("a rule passes when the pack's check would pass it as shipped", () => {
  assert.deepEqual(failing(conformance([row("a")]), "a"), []);
  assert.deepEqual(failing(conformance([row("a", { tp: 0, examples: true })]), "a"), []);
});

test("an unexplained finding on the pack's fixtures or probes fails it, naming the first, as the check fails a shipped rule", () => {
  const withFp = { ...row("a", { fp: 1 }), falsePositives: [{ path: "rerail/probes/A1.py", line: 5, column: 23 }] };
  const all = conformance([withFp, row("b", { fn: 2 }), row("c", { tp: 0 }), row("d", { engineFailures: 1 })]);
  const checks = passesConformanceChecks(all, "a", 1);
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

test("a missed label, a finding on a clean sample or an unread sample fails it; a reviewed report does not; a rule no sample judges is not judged there", () => {
  const judged = (counts: Counts) => failing(conformance([row("a")], { sampleRows: new Map([["a", row("a", counts, "samples")]]) }), "a");
  assert.deepEqual(judged({}), []);
  assert.deepEqual(judged({ reviewed: 3 }), []);
  assert.deepEqual(judged({ fn: 1 }), ["samples-catch-labelled"]);
  assert.deepEqual(judged({ fp: 1 }), ["samples-silent-on-clean"]);
  assert.deepEqual(judged({ engineFailures: 1 }), ["engine-reads-samples"]);
  assert.deepEqual(judged({ syntaxErrorFiles: { count: 1, files: [{ path: "s/a.py", line: 2, column: 1 }] } }), ["engine-reads-samples"]);
  const ids = passesConformanceChecks(conformance([row("a")]), "a", 1).map((check) => check.id);
  assert.ok(!ids.some((id) => id.startsWith("samples-") || id === "engine-reads-samples"));
});

test("stays-silent: an own upstream case upstream and the rule are silent on, or a silent clean sample, meets it, and the detail names each", () => {
  const staysSilent = (own: number, samples: number | undefined) =>
    passesConformanceChecks(conformance([row("a")], samples === undefined ? {} : { sampleRows: new Map([["a", row("a", { cleanSamples: samples, silentCleanSamples: samples }, "samples")]]) }), "a", own)
      .find((check) => check.id === "stays-silent")!;
  assert.deepEqual(staysSilent(3, 2), { id: "stays-silent", ok: true, detail: "silent where it should be: 3 own upstream case(s), 2 clean sample(s)" });
  assert.equal(staysSilent(1, undefined).ok, true);
  assert.equal(staysSilent(0, 1).ok, true);
  assert.equal(staysSilent(0, 0).ok, false);
  assert.equal(staysSilent(0, undefined).ok, false);
});

test("it blocks like any gate, after the V1 gates: a rule that passes them but not the pack's check stops there", () => {
  const pass = { pass: true };
  const fail = { pass: false };
  assert.deepEqual(verdict({ loads: pass, "not-overfit": pass, compact: pass, "matches-upstream-tests": pass, "passes-conformance": fail }), { pass: false, stoppedAt: "passes-conformance" });
  assert.deepEqual(verdict({ loads: pass, "not-overfit": fail, compact: pass, "matches-upstream-tests": pass, "passes-conformance": fail }), { pass: false, stoppedAt: "not-overfit" });
});
