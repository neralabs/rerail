import assert from "node:assert/strict";
import { test } from "vitest";

import { passesConformanceChecks, type PackConformance } from "../../../conformance/port-readiness/checks/passes-conformance.js";
import type { RuleRow } from "../../../conformance/languages/report.js";
import { verdict } from "../../../conformance/port-readiness/types.js";

function row(rule: string, counts: Partial<Pick<RuleRow, "tp" | "fp" | "fn" | "engineFailures" | "examples">> = {}): RuleRow {
  return {
    rule, candidate: true, examples: false, upstream: { tool: "ruff", rule: "X1" }, capabilities: [], source: "fixtures", tp: 2, fp: 0, fn: 0,
    precision: 1, recall: 1, unanalysed: 0, engineFailures: 0, partialFiles: 0, syntaxErrorFiles: { count: 0, files: [] }, divergences: [],
    falsePositives: [], falseNegatives: [], ...counts,
  } as RuleRow;
}

function conformance(rows: RuleRow[], extra: Partial<PackConformance> = {}): PackConformance {
  return { rows: new Map(rows.map((entry) => [entry.rule, entry])), pinsCorpora: false, stale: new Map(), uncovered: new Map(), ruleErrors: new Map(), errors: [], ...extra };
}

const failing = (checks: { id: string; ok: boolean }[]) => checks.filter((check) => !check.ok).map((check) => check.id);

test("a rule passes when the pack's check would pass it as shipped", () => {
  assert.deepEqual(failing(passesConformanceChecks(conformance([row("a")]), "a")), []);
  assert.deepEqual(failing(passesConformanceChecks(conformance([row("a", { tp: 0, examples: true })]), "a")), []);
});

test("an unexplained finding on the pack's fixtures or probes fails it, naming the first, as the check fails a shipped rule", () => {
  const withFp = { ...row("a", { fp: 1 }), falsePositives: [{ path: "rerail/probes/A1.py", line: 5, column: 23 }] } as RuleRow;
  const checks = passesConformanceChecks(conformance([withFp, row("b", { fn: 2 }), row("c", { tp: 0 }), row("d", { engineFailures: 1 })]), "a");
  assert.deepEqual(failing(checks), ["no-unexplained-false-positives"]);
  assert.match(checks.find((check) => !check.ok)!.detail, /1 unexplained FP on the pack's fixtures, first rerail\/probes\/A1\.py:5:23/u);
  const all = conformance([withFp, row("b", { fn: 2 }), row("c", { tp: 0 }), row("d", { engineFailures: 1 })]);
  assert.deepEqual(failing(passesConformanceChecks(all, "b")), ["no-unexplained-misses"]);
  assert.deepEqual(failing(passesConformanceChecks(all, "c")), ["catches-something"]);
  assert.deepEqual(failing(passesConformanceChecks(all, "d")), ["engine-reads-fixtures"]);
});

test("a stale divergence, an uncovered corpus recording or a missing row fails it", () => {
  assert.deepEqual(failing(passesConformanceChecks(conformance([row("a")], { stale: new Map([["a", ["x.py:3 (upstream-wrong)"]]]) }), "a")), ["divergences-current"]);
  const uncovered = conformance([row("a")], { pinsCorpora: true, uncovered: new Map([["a", ["the ruff recording of corpus django does not cover X1"]]]) });
  assert.deepEqual(failing(passesConformanceChecks(uncovered, "a")), ["corpus-recorded"]);
  assert.deepEqual(failing(passesConformanceChecks(conformance([]), "missing")), ["fixtures-row"]);
});

test("it blocks like any gate, after the V1 gates: a rule that passes them but not the pack's check stops there", () => {
  const pass = { pass: true };
  const fail = { pass: false };
  assert.deepEqual(verdict({ loads: pass, "not-overfit": pass, compact: pass, "matches-upstream-tests": pass, "passes-conformance": fail }), { pass: false, stoppedAt: "passes-conformance" });
  assert.deepEqual(verdict({ loads: pass, "not-overfit": fail, compact: pass, "matches-upstream-tests": pass, "passes-conformance": fail }), { pass: false, stoppedAt: "not-overfit" });
});
