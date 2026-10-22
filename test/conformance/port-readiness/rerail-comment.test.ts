import assert from "node:assert/strict";
import { test } from "vitest";

import { COMMENT_LIMIT, COMMENT_MARKER, renderComment, renderJobSummary } from "../../../conformance/port-readiness/comment.js";
import { SUMMARY_FORMAT, type OtherFailure, type RuleVerdict, type StepId, type Summary } from "../../../conformance/port-readiness/summary.js";

/** A port of ruff unless `pack` is complexity, whose rules are not ports. */
function verdict(id: string, location: RuleVerdict["location"], stoppedAt: StepId | null, pack = "python"): RuleVerdict {
  const port = pack !== "complexity";
  return {
    id, file: `packs/${pack}/${location === "shipped" ? "rules" : "candidates/rules"}/${id}.json`, pack, location, port,
    upstream: port ? { tool: "ruff", rule: id.toUpperCase() } : null, stoppedAt,
    check: stoppedAt === null ? "" : "identical-results", why: stoppedAt === null ? "" : `1 upstream test file differs ${"x".repeat(400)}`,
    ...(port ? { onByDefault: true } : {}),
  };
}

/** 100 shipped ruff ports, `failing` of them failing; `candidates` candidates, the first ready to promote, the rest across the gates. */
function summary(candidates: number, failing: number, extra: RuleVerdict[] = [], otherFailures: OtherFailure[] = []): Summary {
  const steps: StepId[] = ["compact", "passes-conformance", "not-overfit", "matches-upstream-tests"];
  const rules = [
    ...Array.from({ length: 100 }, (_, index) => verdict(`ruff-shipped-${index}`, "shipped", index < failing ? "matches-upstream-tests" : null)),
    ...Array.from({ length: candidates }, (_, index) => verdict(`ruff-candidate-${index}-with-a-long-descriptive-name`, "candidate", index === 0 ? null : steps[index % steps.length]!)),
    ...extra,
  ];
  const shipped = rules.filter((rule) => rule.location === "shipped");
  return {
    format: SUMMARY_FORMAT,
    rules,
    shipped: { rules: shipped.length, pass: shipped.filter((rule) => rule.stoppedAt === null).length },
    failing: shipped.filter((rule) => rule.stoppedAt !== null),
    otherFailures,
    promotable: rules.filter((rule) => rule.location === "candidate" && rule.stoppedAt === null),
    notJudged: [],
    pass: failing === 0 && otherFailures.length === 0,
  };
}

const ascii = (text: string) => assert.doesNotMatch(text, /[^\x20-\x7e\n]/u, "non-ASCII character");

test("the comment starts with the marker CI finds earlier comments by, and leads with the shipped count", () => {
  const text = renderComment(summary(10, 2), { commit: "0123456789abcdef", runUrl: "https://example.test/run/1" });
  assert.ok(text.startsWith(COMMENT_MARKER));
  assert.match(text, /^## Rule gates: \[FAIL\] 98\/100 shipped rules pass their required gates; 1 candidate ready to promote$/mu);
  assert.match(text, /^At `0123456789` - \[job summary and reports\]\(https:\/\/example\.test\/run\/1\)$/mu);
  assert.match(text, /^- \*\*Conformance\*\* checks every rule\.$/mu);
  assert.match(text, /^- \*\*Port readiness\*\* checks ports, once they pass conformance\. A port is a rule that comes from an upstream linter/mu);
  ascii(text);
});

test("each failing shipped rule is one line: where it comes from, the stage and gate it stopped at, and why", () => {
  const text = renderComment(summary(10, 2));
  const section = text.slice(text.indexOf("### Shipped rules that fail (2)"), text.indexOf("### Ready to promote"));
  assert.match(section, /^- `ruff-shipped-0` \(python, port of ruff RUFF-SHIPPED-0\): \*\*Port readiness > Matches upstream on its own tests\*\*: 1 upstream test file differs x+\.\.\.$/mu);
  assert.doesNotMatch(section, /^\|/mu);
  assert.doesNotMatch(renderComment(summary(10, 0)), /### Shipped rules that fail/u);
});

test("a blocking failure no rule owns is listed on its own, and counted in the heading", () => {
  const text = renderComment(summary(10, 0, [], [{ what: "python/ruff", check: "report", why: "its port readiness run left no report" }]));
  assert.match(text, /^## Rule gates: \[FAIL\] 100\/100 shipped rules pass their required gates; 1 other blocking failure; 1 candidate ready to promote$/mu);
  assert.match(text, /^### Other blocking failures \(1\)$\n\n^- python\/ruff: report: its port readiness run left no report$/mu);
});

test("ready to promote lists ports and rules that are not ports, then the shipped rules per pack, collapsed", () => {
  const text = renderComment(summary(10, 0, [verdict("complexity-a", "candidate", null, "complexity"), verdict("complexity-b", "shipped", null, "complexity")]));
  assert.match(text, /^## Rule gates: \[PASS\] 101\/101 shipped rules pass their required gates; 2 candidates ready to promote$/mu);
  const section = text.slice(text.indexOf("### Ready to promote (2)"), text.indexOf("<summary>Candidates that fail a gate"));
  assert.match(section, /^- `ruff-candidate-0-with-a-long-descriptive-name` \(python, port of ruff RUFF-CANDIDATE-0-WITH-A-LONG-DESCRIPTIVE-NAME\): on by default$/mu);
  assert.match(section, /^- `complexity-a` \(complexity\)$/mu);
  assert.match(section, /<summary>Shipped rules per pack \(101\)<\/summary>\n\n\| Rule pack \| Shipped rules passing \|\n\| --- \| --- \|\n\| complexity \| 1\/1 \|\n\| python \| 100\/100 \|/u);
  assert.match(renderComment(summary(0, 0)), /^### Ready to promote \(0\)\n\nNo candidate is ready to promote yet\.$/mu);
});

test("the failing candidates are collapsed, grouped by stage and gate in the order a rule meets them", () => {
  const text = renderComment(summary(9, 0));
  const section = text.slice(text.indexOf("<summary>Candidates that fail a gate (8)</summary>"));
  const groups = [...section.matchAll(/^<summary>(.+)<\/summary>$/gmu)].map((match) => match[1]);
  assert.deepEqual(groups, [
    "Candidates that fail a gate (8)",
    "Conformance > Compact (no written-out expansion): 2",
    "Conformance > Passes the pack's conformance check: 2",
    "Port readiness > Not overfit to upstream's tests: 2",
    "Port readiness > Matches upstream on its own tests: 2",
  ]);
  assert.match(section, /^- `ruff-candidate-4-with-a-long-descriptive-name` \(python, port of ruff RUFF-CANDIDATE-4-WITH-A-LONG-DESCRIPTIVE-NAME\): 1 upstream test file differs x+\.\.\.$/mu);
});

test("thousands of candidates still fit in one comment: lists shrink, counts stay", () => {
  const text = renderComment(summary(3000, 0));
  assert.ok(text.length <= COMMENT_LIMIT, `${text.length} characters`);
  assert.match(text, /<summary>Candidates that fail a gate \(2999\)<\/summary>/u);
  assert.match(text, /more: see the job summary|^- Conformance > Compact \(no written-out expansion\): 750$/mu);
});

test("the job summary is the comment's page with every list in full", () => {
  const text = renderJobSummary(summary(300, 60));
  assert.ok(!text.startsWith(COMMENT_MARKER));
  assert.doesNotMatch(text, /see the job summary/u);
  assert.equal(text.match(/^- `ruff-shipped-\d+`/gmu)?.length, 60);
  assert.match(text, /^- `ruff-candidate-299-with-a-long-descriptive-name` .*x{400}$/mu);
  ascii(text);
});

test("a pull request that re-ran only some upstream linters says which it left out", () => {
  const text = renderComment({ ...summary(10, 0), notJudged: ["python/bandit", "javascript/eslint"] });
  assert.match(text, /^Not re-run on this change, since nothing their ports read changed: python\/bandit, javascript\/eslint\. Main's results stand for them, and the counts leave them out\.$/mu);
  assert.doesNotMatch(renderComment(summary(10, 0)), /Not re-run/u);
});
