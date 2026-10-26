import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "vitest";

import { COMMENT_LIMIT, COMMENT_MARKER, renderComment } from "../../../conformance/port-readiness/comment.js";
import { readRecordingSummaries, type Summary } from "../../../conformance/port-readiness/summary.js";

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

test("the recording summaries a run's legs wrote are read beside the reports, sorted by tool-pack", () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-comment-"));
  try {
    writeFileSync(join(directory, "recording-sql--sqlfluff.md"), "## Port readiness recordings: ✅ re-recorded, and upstream answers the same\n");
    writeFileSync(join(directory, "recording-python--ruff.md"), "## Port readiness recordings: 1 tool-pack moved\n");
    writeFileSync(join(directory, "python--ruff.json"), "{}\n");
    writeFileSync(join(directory, "recording-notes.md"), "not a leg's summary\n");
    assert.deepEqual(readRecordingSummaries(directory).map((entry) => entry.toolPack), ["python/ruff", "sql/sqlfluff"]);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test("a tool-pack this run recorded gets its per-rule summary in the comment, collapsed under what it moved", () => {
  const markdown = [
    "## Port readiness recordings: 1 tool-pack moved",
    "",
    "Upstream's recorded answers are not in the diff.",
    "",
    "### `python/ruff`",
    "",
    "| Upstream rule | | Expected findings | Facts |",
    "| --- | --- | --- | --- |",
    "| `E501` | changed | +2 / −0 | — |",
    "",
  ].join("\n");
  const text = renderComment(summary(10, 0), { recordings: [{ toolPack: "python/ruff", markdown }] });
  const section = text.slice(text.indexOf("### Recorded by this run"), text.indexOf("### Ready to promote"));
  assert.match(section, /<summary>python\/ruff: 1 tool-pack moved<\/summary>/u);
  assert.match(section, /^\| `E501` \| changed \| \+2 \/ −0 \| — \|$/mu);
  assert.doesNotMatch(section, /^## Port readiness recordings/mu);
  assert.doesNotMatch(renderComment(summary(10, 0)), /Recorded by this run/u);
});

test("recording summaries are cut to fit, and the tool-packs past the budget are named", () => {
  const long = `## Port readiness recordings: 1 tool-pack moved\n\n${"| `X` | changed | +1 / −0 | — |\n".repeat(1000)}`;
  const recordings = ["a/one", "b/two", "c/three", "d/four"].map((toolPack) => ({ toolPack, markdown: long }));
  const text = renderComment(summary(10, 0), { recordings });
  assert.ok(text.length <= COMMENT_LIMIT, `${text.length} characters`);
  assert.match(text, /Cut short: the leg's job summary has all of it\./u);
  assert.match(text, /^Also recorded, too long to show here \(see each leg's job summary\): c\/three, d\/four\.$/mu);
});
