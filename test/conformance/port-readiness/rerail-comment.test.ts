import assert from "node:assert/strict";
import { test } from "vitest";

import { COMMENT_LIMIT, COMMENT_MARKER, renderComment } from "../../../conformance/port-readiness/comment.js";
import type { Summary } from "../../../conformance/port-readiness/summary.js";

function summary(candidates: number, blocking: number): Summary {
  const gates = ["Compiles and loads on this engine", "Not overfit to upstream's tests", "Matches upstream on its own tests"];
  return {
    format: "rerail.port-readiness-summary/v1",
    shipped: { rules: 100, pass: 100 - blocking },
    candidates: { rules: candidates, pass: 1 },
    blocking: Array.from({ length: blocking }, (_, index) => ({ pack: "python/ruff", rule: `ruff-shipped-${index}`, stoppedAt: gates[2]!, check: "identical-results", why: `1 upstream test file(s) differ, e.g. fixtures/x${index}.py: missing 3:1` })),
    engineGaps: [],
    ungated: { checks: [], notGated: [{ pack: "complexity", rules: 3 }], pending: [{ toolPack: "javascript/eslint", reason: "pending" }] },
    missingReports: [],
    notJudged: [],
    candidateFunnel: { "python/ruff": { [gates[0]!]: 1, [gates[1]!]: 2, [gates[2]!]: candidates - 4, "passes every gate": 1 } },
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
  assert.match(text, /❌ 2 blocking failures/u);
  assert.match(text, /`ruff-shipped-1`/u);
  assert.match(text, /at `0123456789`/u);
  assert.match(text, /\[job summary and reports\]\(https:\/\/example\.test\/run\/1\)/u);
});

test("thousands of candidates still fit in one comment: lists shrink, counts stay", () => {
  const text = renderComment(summary(3000, 0));
  assert.ok(text.length <= COMMENT_LIMIT, `${text.length} characters`);
  assert.match(text, /✅ every shipped rule passes/u);
  assert.match(text, /python\/ruff: 2999 candidates stop at a gate/u);
  assert.match(text, /more: see the job summary|Too many to list here/u);
});

test("candidates that pass every gate are called out, outside any collapsed section", () => {
  const text = renderComment(summary(3000, 0));
  assert.match(text, /🟢 1 candidate ready to promote/u);
  const section = text.slice(text.indexOf("### Ready to promote"), text.indexOf("### Candidates"));
  assert.match(section, /`ruff-candidate-0-with-a-long-descriptive-name` \| X100 \| on/u);
  assert.doesNotMatch(section, /<details>/u);
});

test("a pull request that judged only some tool-packs says which it left out, and what the counts cover", () => {
  const text = renderComment({ ...summary(10, 0), notJudged: ["python/bandit", "javascript/eslint"] });
  assert.match(text, /^## Port readiness: ✅ every shipped rule passes · 🟢 1 candidate ready to promote · in the 1 of 3 tool-packs judged$/mu);
  assert.match(text, /^Not judged on this change: python\/bandit, javascript\/eslint \(nothing they read changed against the base, so main's verdict stands\)\.$/mu);
  assert.doesNotMatch(renderComment(summary(10, 0)), /judged/u);
});

test("candidates with no conformance entry are listed as not judged, apart from the gate funnel", () => {
  const text = renderComment({ ...summary(10, 0), unmappedCandidates: [{ pack: "python", file: "packs/python/candidates/rules/ruff-a005-stdlib-module-shadowing.json" }] });
  const section = text.slice(text.indexOf("### Candidates"));
  assert.match(section, /<summary>1 candidate is not judged: no conformance\.json entry maps it to an upstream rule, so no gate ran<\/summary>/u);
  assert.match(section, /^- python: `packs\/python\/candidates\/rules\/ruff-a005-stdlib-module-shadowing\.json`$/mu);
});

test("a tool-pack that needs recording again gets a line naming the label, and none does without one", () => {
  const base = summary(10, 2);
  const flagged = (toolPack: string) => ({ ...base.toolPacks[0]!, toolPack, needsRecording: true });
  const text = renderComment({ ...base, toolPacks: [flagged("python/ruff")] });
  assert.match(text, /^`python\/ruff` needs recording again: in a pull request, a maintainer adds the `record-port-readiness` label \(or runs `port-readiness-record\.yml` by hand with the pull request's number and head sha\), and that run comments on the pull request with what comes next; on `main`, a maintainer runs `port-readiness-record\.yml` from `main` with `pr` and `sha` empty\.$/mu);
  assert.doesNotMatch(renderComment(base), /needs recording again/u);
  assert.match(renderComment({ ...base, toolPacks: [flagged("python/ruff"), flagged("python/bandit")] }), /^`python\/ruff` and `python\/bandit` need recording again: /mu);
});
