import assert from "node:assert/strict";
import { test } from "vitest";

import { COMMENT_LIMIT, COMMENT_MARKER, renderComment } from "../../../conformance/port-readiness/comment.js";
import type { Summary } from "../../../conformance/port-readiness/summary.js";

function conformance(blocking = 0): Summary["conformance"] {
  return {
    shipped: { rules: 103, pass: 103 - blocking },
    candidates: { rules: 12, pass: 10 },
    packs: [{ pack: "complexity", shipped: `${3 - blocking}/3`, candidates: "0/0" }, { pack: "python", shipped: "100/100", candidates: "8/10" }, { pack: "sql", shipped: "0/0", candidates: "2/2" }],
    blocking: Array.from({ length: blocking }, (_, index) => ({ pack: "complexity", rule: `complexity-${index}`, stoppedAt: "Compact (no written-out expansion)", check: "expansion", why: "44.6% of the clauses is structure written out over and over (limit 33.3%)" })),
    funnel: { python: { "Compiles and loads on this engine": 1, "Compact (no written-out expansion)": 0, "Passes the pack's conformance check": 1, "passes every conformance gate": 8 } },
    candidateRows: [
      { pack: "python", rule: "ruff-candidate-1-with-a-long-descriptive-name", stoppedAt: "Passes the pack's conformance check", check: "fixtures-row", why: "the pack's conformance check scored no fixtures for it" },
      { pack: "python", rule: "ruff-candidate-2-with-a-long-descriptive-name", stoppedAt: "Compiles and loads on this engine", check: "schema", why: "/clauses: Required" },
    ],
    notApplicable: [{ pack: "complexity", file: "packs/complexity/rules/complexity-0.json", reason: "packs/complexity has no conformance.json" }],
  };
}

function summary(candidates: number, blocking: number): Summary {
  const gates = ["Not overfit to upstream's tests", "Matches upstream on its own tests", "Fails a conformance gate"];
  return {
    format: "rerail.port-readiness-summary/v2",
    conformance: conformance(),
    shipped: { rules: 100, pass: 100 - blocking },
    candidates: { rules: candidates, pass: 1 },
    blocking: Array.from({ length: blocking }, (_, index) => ({ pack: "python/ruff", rule: `ruff-shipped-${index}`, stoppedAt: gates[2]!, check: "identical-results", why: `1 upstream test file(s) differ, e.g. fixtures/x${index}.py: missing 3:1` })),
    engineGaps: [],
    ungated: { checks: [], notGated: [{ pack: "complexity", rules: 3 }], pending: [{ toolPack: "javascript/eslint", reason: "pending" }] },
    missingReports: [],
    notJudged: [],
    candidateFunnel: { "python/ruff": { [gates[0]!]: 1, [gates[1]!]: candidates - 4, "Tool-pack checks": 0, [gates[2]!]: 2, "Ready to promote": 1 } },
    candidateRows: Array.from({ length: candidates }, (_, index) => ({
      toolPack: "python/ruff",
      pack: "python",
      rule: `ruff-candidate-${index}-with-a-long-descriptive-name`,
      stoppedAt: index === 0 ? "passes" : gates[index % 3]!,
      check: "identical-results",
      why: "x".repeat(400),
    })),
    unmappedCandidates: [],
    promotable: [{ toolPack: "python/ruff", rule: "ruff-candidate-0-with-a-long-descriptive-name", upstreamRule: "X100", startsOn: true, startReason: "on in default" }],
    onOff: [],
    toolPacks: [{ toolPack: "python/ruff", shipped: `${100 - blocking}/100`, candidates: `1/${candidates}`, failedGate: {}, integrity: [], gapCandidates: 0, toolPackChecks: true }],
    pass: blocking === 0,
  } as unknown as Summary;
}

test("the comment starts with the marker CI finds earlier comments by, and names the blockers", () => {
  const text = renderComment(summary(10, 2), { commit: "0123456789abcdef", runUrl: "https://example.test/run/1" });
  assert.ok(text.startsWith(COMMENT_MARKER));
  assert.match(text, /^## Conformance and port readiness: ❌ 2 blocking failures · 🟢 1 candidate ready to promote$/mu);
  assert.match(text, /^### Port readiness: ❌ 2 blocking failures · 🟢 1 candidate ready to promote$/mu);
  assert.match(text, /^### Conformance: ✅ every shipped rule passes$/mu);
  assert.match(text, /`ruff-shipped-1`/u);
  assert.match(text, /at `0123456789`/u);
  assert.match(text, /\[job summary and reports\]\(https:\/\/example\.test\/run\/1\)/u);
});

test("thousands of candidates still fit in one comment: lists shrink, counts stay", () => {
  const text = renderComment(summary(3000, 0));
  assert.ok(text.length <= COMMENT_LIMIT, `${text.length} characters`);
  assert.match(text, /✅ every shipped rule passes/u);
  // Port readiness lists the candidates that stop at a port gate; the ones that fail a conformance gate are Conformance's.
  assert.match(text, /python\/ruff: 1999 candidates stop at a port gate/u);
  assert.match(text, /more: see the job summary|Too many to list here/u);
});

test("candidates that pass every gate are called out, outside any collapsed section", () => {
  const text = renderComment(summary(3000, 0));
  assert.match(text, /🟢 1 candidate ready to promote/u);
  const section = text.slice(text.indexOf("#### Ready to promote"), text.indexOf("#### Candidates"));
  assert.match(section, /`ruff-candidate-0-with-a-long-descriptive-name` \| X100 \| on/u);
  assert.doesNotMatch(section, /<details>/u);
});

test("a pull request that judged only some tool-packs says which it left out, and what the counts cover", () => {
  const text = renderComment({ ...summary(10, 0), notJudged: ["python/bandit", "javascript/eslint"] });
  assert.match(text, /^### Port readiness: ✅ every shipped rule passes · 🟢 1 candidate ready to promote · in the 1 of 3 tool-packs judged$/mu);
  assert.match(text, /^The rules of the tool-packs not judged on this change \(python\/bandit, javascript\/eslint\) keep main's verdict here too\.$/mu);
  assert.match(text, /^Not judged on this change: python\/bandit, javascript\/eslint \(nothing they read changed against the base, so main's verdict stands\)\.$/mu);
  assert.doesNotMatch(renderComment(summary(10, 0)), /judged/u);
});

test("candidates with no conformance entry are listed as not judged, apart from the gate funnel", () => {
  const text = renderComment({ ...summary(10, 0), unmappedCandidates: [{ pack: "python", file: "packs/python/candidates/rules/ruff-a005-stdlib-module-shadowing.json" }] });
  const section = text.slice(text.indexOf("#### Candidates"));
  assert.match(section, /<summary>1 candidate is not judged for port readiness: no conformance\.json entry maps it to an upstream rule, so no port gate ran \(the conformance gates judge it\)<\/summary>/u);
  assert.match(section, /^- python: `packs\/python\/candidates\/rules\/ruff-a005-stdlib-module-shadowing\.json`$/mu);
});

test("the conformance section comes first and covers every pack, ported or not, with its own blockers", () => {
  const text = renderComment({ ...summary(10, 0), conformance: conformance(1), pass: false });
  assert.match(text, /^## Conformance and port readiness: ❌ 1 blocking failure · 🟢 1 candidate ready to promote$/mu);
  assert.ok(text.indexOf("### Conformance") < text.indexOf("### Port readiness"));
  const section = text.slice(text.indexOf("### Conformance"), text.indexOf("### Port readiness"));
  assert.match(section, /^### Conformance: ❌ 1 blocking failure$/mu);
  assert.match(section, /^\| complexity \| 2\/3 \| 0\/0 \|$/mu);
  assert.match(section, /^\| \*\*All\*\* \| \*\*102\/103\*\* \| \*\*10\/12\*\* \|$/mu);
  assert.match(section, /^\| complexity \| `complexity-0` \| Compact \(no written-out expansion\) \| expansion \|/mu);
  assert.match(section, /does not apply to 1 rule \(complexity\)/u);
  assert.match(section, /<summary>python: 2 candidates fail a conformance gate<\/summary>/u);
  // Grouped in the order the gates run, whatever order the rows come in.
  assert.ok(section.indexOf("**Compiles and loads on this engine** (1)") < section.indexOf("**Passes the pack's conformance check** (1)"));
  // A port gate never shows in the conformance section, nor a conformance blocker in port readiness.
  assert.doesNotMatch(section, /Not overfit|Matches upstream on its own tests/u);
  assert.doesNotMatch(text.slice(text.indexOf("### Port readiness")), /`complexity-0`/u);
});

test("with no tool-pack judged, port readiness is one line", () => {
  const text = renderComment({ ...summary(0, 0), toolPacks: [], candidateFunnel: {}, candidateRows: [], promotable: [], shipped: { rules: 0, pass: 0 }, candidates: { rules: 0, pass: 0 }, notJudged: ["python/ruff"] });
  const section = text.slice(text.indexOf("### Port readiness"));
  assert.match(section, /^Not judged on this change: python\/ruff/mu);
  assert.doesNotMatch(section, /#### /u);
});
