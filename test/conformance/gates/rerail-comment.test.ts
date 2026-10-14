import assert from "node:assert/strict";
import { test } from "vitest";

import { COMMENT_LIMIT, COMMENT_MARKER, renderComment } from "../../../conformance/gates/comment.js";
import type { Summary } from "../../../conformance/gates/summary.js";

function summary(candidates: number, blocking: number): Summary {
  const gates = ["Compiles and loads on this engine", "Not overfit to upstream's tests", "Matches upstream on its own tests"];
  return {
    format: "rerail.rule-gates-summary/v1",
    shipped: { rules: 100, pass: 100 - blocking },
    candidates: { rules: candidates, pass: 1 },
    blocking: Array.from({ length: blocking }, (_, index) => ({ pack: "python/ruff", rule: `ruff-shipped-${index}`, stoppedAt: gates[2]!, check: "identical-results", why: `1 upstream test file(s) differ, e.g. fixtures/x${index}.py: missing 3:1` })),
    engineGaps: [],
    ungated: { checks: [], notGated: [{ pack: "complexity", rules: 3 }], pending: [{ toolPack: "javascript/eslint", reason: "pending" }] },
    missingReports: [],
    candidateFunnel: { "python/ruff": { [gates[0]!]: 1, [gates[1]!]: 2, [gates[2]!]: candidates - 4, "passes every gate": 1 } },
    candidateRows: Array.from({ length: candidates }, (_, index) => ({
      toolPack: "python/ruff",
      pack: "python",
      rule: `ruff-candidate-${index}-with-a-long-descriptive-name`,
      stoppedAt: index === 0 ? "passes" : gates[index % 3]!,
      check: "identical-results",
      why: "x".repeat(400),
    })),
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
