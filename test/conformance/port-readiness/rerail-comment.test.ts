import assert from "node:assert/strict";
import { test } from "vitest";

import { COMMENT_LIMIT, COMMENT_MARKER, renderComment } from "../../../conformance/port-readiness/comment.js";
import { SUMMARY_FORMAT, type RuleVerdict, type StepId, type Summary } from "../../../conformance/port-readiness/summary.js";

function verdict(id: string, source: string, location: RuleVerdict["location"], stoppedAt: StepId | null, gated = !source.includes("(") && source !== "complexity" && source !== "sql/sqlfluff"): RuleVerdict {
  const pack = source.split(/[/ ]/u)[0]!;
  return { id, file: `packs/${pack}/${location === "shipped" ? "rules" : "candidates/rules"}/${id}.json`, pack, source, gated, location, stoppedAt, check: stoppedAt === null ? "" : "identical-results", why: stoppedAt === null ? "" : "x".repeat(400) };
}

/** `candidates` ruff candidates (the first ready to promote, the rest across the gates), 100 shipped, `blocking` of them failing. */
function summary(candidates: number, blocking: number, extra: RuleVerdict[] = []): Summary {
  const steps: StepId[] = ["compact", "passes-conformance", "not-overfit", "matches-upstream-tests"];
  const rules = [
    ...Array.from({ length: 100 }, (_, index) => verdict(`ruff-shipped-${index}`, "python/ruff", "shipped", index < blocking ? "matches-upstream-tests" : null)),
    ...Array.from({ length: candidates }, (_, index) => verdict(`ruff-candidate-${index}-with-a-long-descriptive-name`, "python/ruff", "candidate", index === 0 ? null : steps[index % steps.length]!)),
    ...extra,
  ];
  return {
    format: SUMMARY_FORMAT,
    rules,
    shipped: { rules: rules.filter((rule) => rule.location === "shipped").length, pass: rules.filter((rule) => rule.location === "shipped" && rule.stoppedAt === null).length },
    blocking: rules.filter((rule) => rule.location === "shipped" && rule.stoppedAt !== null).map((rule) => ({ source: rule.source, rule: rule.id, stoppedAt: "Matches upstream on its own tests", check: rule.check, why: "1 upstream test file(s) differ, e.g. fixtures/x.py: missing 3:1" })),
    engineGaps: [],
    changed: null,
    promotable: candidates === 0 ? [] : [{ toolPack: "python/ruff", rule: "ruff-candidate-0-with-a-long-descriptive-name", upstreamRule: "X100", startsOn: true, startReason: "on in default" }],
    notApplicable: [],
    ungated: { checks: [], notGated: [{ pack: "complexity", rules: 3 }], pending: [] },
    missingReports: [],
    notJudged: [],
    onOff: [],
    toolPacks: [],
    pass: blocking === 0,
  };
}

test("the comment starts with the marker CI finds earlier comments by, and leads with the blockers", () => {
  const text = renderComment(summary(10, 2), { commit: "0123456789abcdef", runUrl: "https://example.test/run/1" });
  assert.ok(text.startsWith(COMMENT_MARKER));
  assert.match(text, /^## Rule gates: ❌ 2 blocking failures · 🟢 1 ready to promote$/mu);
  assert.match(text, /^At `0123456789` · \[job summary and reports\]\(https:\/\/example\.test\/run\/1\)$/mu);
  const blockers = text.slice(text.indexOf("### Blocking failures"), text.indexOf("<details>"));
  assert.match(blockers, /^\| `ruff-shipped-1` \| python\/ruff \| Matches upstream on its own tests \| identical-results \|/mu);
});

test("the verdict comes first, the standing last and collapsed", () => {
  const text = renderComment(summary(10, 0));
  assert.match(text, /^## Rule gates: ✅ every shipped rule passes \(100\) · 🟢 1 ready to promote$/mu);
  assert.doesNotMatch(text, /### Blocking failures/u);
  assert.ok(text.indexOf("### Ready to promote") < text.indexOf("<summary>Where all 10 candidates stand"));
  const ready = text.slice(text.indexOf("### Ready to promote"), text.indexOf("<details>"));
  assert.match(ready, /^\| `ruff-candidate-0-with-a-long-descriptive-name` \| python\/ruff \| X100 \| on \|$/mu);
});

test("each candidate is counted once: the funnel and the per-source rows add up to the candidates", () => {
  const text = renderComment(summary(9, 0, [verdict("clippy-a", "rust/clippy", "candidate", null, false), verdict("clippy-b", "rust/clippy", "candidate", "compact", false)]));
  assert.match(text, /<summary>Where all 11 candidates stand: 9 of gated tool-packs, 2 that no gated tool-pack judges<\/summary>/u);
  // Funnel rows: 9 ruff candidates (1 ready, then 2 at each of the four gates), 2 clippy (1 at compact, 1 passing).
  assert.match(text, /^\| Compact \(no written-out expansion\) \| conformance \| 2 \| 1 \|$/mu);
  assert.match(text, /^\| Not overfit to upstream's tests \| port \| 2 \| — \|$/mu);
  assert.match(text, /^\| \*\*Passes every gate that runs\*\* \|  \| \*\*1\*\* ready to promote \| \*\*1\*\* pass the conformance gates \|$/mu);
  assert.match(text, /^\| rust\/clippy \| 0\/0 \| 2 \| 1 \| 1 \|$/mu);
});

test("the rules a pull request changes are listed with their verdicts, blockers first", () => {
  const changed = [verdict("ruff-candidate-2-with-a-long-descriptive-name", "python/ruff", "candidate", "not-overfit"), verdict("ruff-shipped-0", "python/ruff", "shipped", "matches-upstream-tests")];
  const text = renderComment({ ...summary(10, 1), changed: { verdicts: changed, notJudged: [], removed: ["packs/python/candidates/rules/gone.json"] } });
  const section = text.slice(text.indexOf("### Rules this pull request changes"), text.indexOf("### Ready to promote"));
  assert.match(section, /^### Rules this pull request changes \(2\)$/mu);
  assert.match(section, /^1 shipped rule: 1 block · 1 candidate: 1 stop at a gate$/mu);
  assert.ok(section.indexOf("`ruff-shipped-0`") < section.indexOf("`ruff-candidate-2"));
  assert.match(section, /\| ❌ blocks at Matches upstream on its own tests: identical-results: x+…/u);
  assert.match(section, /^1 rule document removed or moved away\.$/mu);
  assert.match(renderComment({ ...summary(10, 0), changed: { verdicts: [], notJudged: [], removed: [] } }), /^None: it changes no rule document\.$/mu);
  assert.doesNotMatch(renderComment(summary(10, 0)), /Rules this pull request changes/u);
});

test("thousands of candidates still fit in one comment: lists shrink, counts stay", () => {
  const text = renderComment(summary(3000, 0));
  assert.ok(text.length <= COMMENT_LIMIT, `${text.length} characters`);
  assert.match(text, /<summary>Where all 3000 candidates stand/u);
  assert.match(text, /more: see the job summary|Each candidate's verdict: see the job summary/u);
});

test("a pull request that judged only some tool-packs says which it left out", () => {
  const text = renderComment({ ...summary(10, 0), notJudged: ["python/bandit", "javascript/eslint"] });
  assert.match(text, /^Not judged on this change: python\/bandit, javascript\/eslint \(nothing they read changed against the base, so main's verdict stands; the counts leave them out\)\.$/mu);
  assert.doesNotMatch(renderComment(summary(10, 0)), /Not judged on this change/u);
});

test("the rules passes-conformance does not apply to are named by why", () => {
  const text = renderComment({
    ...summary(10, 0),
    notApplicable: [
      { source: "complexity", file: "packs/complexity/rules/a.json", reason: "" },
      { source: "complexity", file: "packs/complexity/rules/b.json", reason: "" },
      { source: "python (no conformance.json entry)", file: "packs/python/candidates/rules/x.json", reason: "" },
    ],
  });
  assert.match(text, /does not apply to 3 rules, since it has nothing to score them against: complexity's 2 \(the pack has no conformance\.json\) and 1 document no conformance\.json entry maps\./u);
  assert.match(text, /^Frozen in `ungated-rules\.json`, since no port gate can judge them: complexity \(3 shipped rules\)\.$/mu);
});
