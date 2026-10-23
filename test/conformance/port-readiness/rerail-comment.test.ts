import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "vitest";

import { COMMENT_LIMIT, COMMENT_MARKER, renderComment, renderJobSummary } from "../../../conformance/port-readiness/comment.js";
import { failureLabel, failurePoint } from "../../../conformance/port-readiness/failure-points.js";
import { readRecordingSummaries, SUMMARY_FORMAT, type OtherFailure, type RuleVerdict, type StepId, type Summary } from "../../../conformance/port-readiness/summary.js";

/** The check a rule stopping at each step fails in these tests, unless it names its own. */
const CHECKS: Partial<Record<StepId, string>> = { origin: "origin", compact: "no-combinatorial-expansion", fixtures: "catches-something", "not-overfit": "no-fixture-only-names", "upstream-evidence": "fires" };

/**
 * A port of `tool`, unless `tool` is null: then an original (a complexity rule), or with `undeclared` a document that
 * declares no origin. A port holds 2 real-code goldens, a bandit port none.
 */
function verdict(id: string, location: RuleVerdict["location"], stoppedAt: StepId | null, tool: string | null = "ruff", pack = tool === null ? "complexity" : "python", undeclared = false, check?: string): RuleVerdict {
  const failed = check ?? (stoppedAt === null ? "" : CHECKS[stoppedAt]!);
  return {
    id, file: `packs/${pack}/${location === "shipped" ? "rules" : "candidates/rules"}/${id}.json`, pack, location, origin: tool !== null ? "port" : undeclared ? null : "original",
    upstream: tool === null ? null : { tool, rule: id.toUpperCase() }, stoppedAt,
    check: failed, why: stoppedAt === null ? "" : failed === "has-fixtures" ? "no conformance.json entry maps it (the fixtures are GPL-derived)" : `1 upstream test file differs ${"x".repeat(400)}`,
    ...(tool === null ? {} : { onByDefault: true, realCodeGoldens: tool === "bandit" ? 0 : 2 }),
  };
}

function summary(rules: RuleVerdict[], otherFailures: OtherFailure[] = []): Summary {
  const shipped = rules.filter((rule) => rule.location === "shipped");
  return {
    format: SUMMARY_FORMAT,
    rules,
    shipped: { rules: shipped.length, pass: shipped.filter((rule) => rule.stoppedAt === null).length },
    failing: shipped.filter((rule) => rule.stoppedAt !== null),
    otherFailures,
    promotable: rules.filter((rule) => rule.location === "candidate" && rule.stoppedAt === null),
    notJudged: [],
    pass: shipped.every((rule) => rule.stoppedAt === null) && otherFailures.length === 0,
  };
}

/**
 * python: 3 shipped ruff rules (one failing) and 2 bandit; candidates across the steps, among them a port no
 * conformance.json entry maps (ruff-g, so it has no fixtures) and a document that declares no origin (python-h);
 * complexity: originals.
 */
const sample = (): RuleVerdict[] => [
  verdict("ruff-a", "shipped", null),
  verdict("ruff-b", "shipped", null),
  verdict("ruff-c", "shipped", "compact"),
  verdict("bandit-a", "shipped", null, "bandit"),
  verdict("bandit-b", "shipped", null, "bandit"),
  verdict("complexity-a", "shipped", null, null),
  verdict("complexity-b", "shipped", null, null),
  verdict("ruff-d", "candidate", null),
  verdict("ruff-e", "candidate", "fixtures"),
  verdict("ruff-f", "candidate", "fixtures", "ruff", "python", false, "identical-results"),
  verdict("bandit-c", "candidate", "not-overfit", "bandit"),
  verdict("ruff-g", "candidate", "fixtures", "ruff", "python", false, "has-fixtures"),
  verdict("python-h", "candidate", "origin", null, "python", true),
  verdict("ruff-i", "candidate", "upstream-evidence"),
  verdict("complexity-c", "candidate", null, null),
];

/** The `<details>` block whose summary is `title`, up to its end. */
function block(text: string, title: string): string {
  const start = text.indexOf(`<summary>${title}</summary>`);
  assert.ok(start >= 0, `no block ${title}`);
  return text.slice(start, text.indexOf("</details>", start));
}

test("the comment starts with the marker CI finds earlier comments by, and leads with the counts", () => {
  const text = renderComment(summary(sample()), { commit: "0123456789abcdef", runUrl: "https://example.test/run/1" });
  assert.ok(text.startsWith(COMMENT_MARKER));
  assert.match(text, /^## Rule gates: ❌ 6\/7 shipped rules pass · 🟢 2 ready to promote$/mu);
  assert.match(text, /^At `0123456789` · \[job summary and reports\]\(https:\/\/example\.test\/run\/1\)$/mu);
  assert.match(text, /^- \*\*Conformance\*\* checks every rule\.$/mu);
  assert.match(text, /^- \*\*Port readiness\*\* checks ports, once they pass conformance\. A port is a rule that comes from an upstream linter/mu);
  assert.match(renderComment(summary(sample().filter((rule) => rule.id !== "ruff-c"))), /^## Rule gates: ✅ 6\/6 shipped rules pass · 🟢 2 ready to promote$/mu);
});

test("the shipped rule summary lists each blocker, then each pack's shipped rules by origin; a pack with a blocker opens", () => {
  const text = renderComment(summary(sample()));
  const section = text.slice(text.indexOf("### ❌ Shipped Rule Summary: 6/7 pass, 1 blocking"), text.indexOf("### Candidate Rule Summary"));
  assert.match(section, /^- ❌ `ruff-c` \(python, from ruff RUFF-C\): \*\*Conformance: Lists cases one by one instead of expressing the logic\.\*\* 1 upstream test file differs x+\.\.\. \*\*Fix:\*\* rewrite the repeated clauses as one general clause\.$/mu);
  assert.match(section, /<details open>\n<summary>python: 5<\/summary>/u);
  assert.match(section, /<details>\n<summary>complexity: 2<\/summary>/u);
  assert.match(block(section, "python: 5"), /^\| Origin \| ✅ Pass \| ❌ Fail \| Total \|\n\| --- \| --- \| --- \| --- \|\n\| bandit \| 2 \| 0 \| 2 \|\n\| ruff \| 2 \| 1 \| 3 \|\n\| \*\*Total\*\* \| \*\*4\*\* \| \*\*1\*\* \| \*\*5\*\* \|$/mu);
  // One origin needs no total row.
  assert.doesNotMatch(block(section, "complexity: 2"), /Total\*\*/u);
  assert.match(block(section, "complexity: 2"), /^\| original \| 2 \| 0 \| 2 \|$/mu);
  // The ports with no real-code goldens are counted under their pack's table; an original is no port.
  assert.match(block(section, "python: 5"), /\n\nNo real-code evidence: 2 of 5 ports are checked on upstream's tests only\.\n/u);
  assert.doesNotMatch(block(section, "complexity: 2"), /No real-code evidence/u);
});

test("a blocking failure no rule owns is listed with the blockers, and counted in the heading", () => {
  const text = renderComment(summary(sample().filter((rule) => rule.id !== "ruff-c"), [{ what: "python/ruff", check: "report", why: "its port readiness run left no report" }]));
  assert.match(text, /^## Rule gates: ❌ 6\/6 shipped rules pass · 1 other blocking failure · 🟢 2 ready to promote$/mu);
  assert.match(text, /^### ❌ Shipped Rule Summary: 6\/6 pass, 1 blocking$/mu);
  assert.match(text, /^- ❌ python\/ruff: report: its port readiness run left no report$/mu);
});

test("the candidate rule summary lists the candidates ready to promote, then each pack's candidates by origin and gate", () => {
  const text = renderComment(summary(sample()));
  const section = text.slice(text.indexOf("### Candidate Rule Summary: 8 candidates, 🟢 2 ready to promote"));
  assert.match(section, /^- 🟢 `ruff-d` \(python, from ruff RUFF-D\): on by default$/mu);
  assert.match(section, /^- 🟢 `complexity-c` \(complexity, original\)$/mu);
  const python = block(section, "python: 7");
  assert.match(python, /^\| Origin \| 🟢 Ready \| ❌ Conformance \| ❌ Port readiness \| Total \|$/mu);
  assert.match(python, /^\| bandit \| 0 \| 0 \| 1 \| 1 \|\n\| ruff \| 1 \| 3 \| 1 \| 5 \|\n\| not declared \| 0 \| 1 \| 0 \| 1 \|\n\| \*\*Total\*\* \| \*\*1\*\* \| \*\*4\*\* \| \*\*2\*\* \| \*\*7\*\* \|$/mu);
  assert.match(python, /^\| Fails because \| bandit \| ruff \| not declared \|$/mu);
  // Every failure is named by its check: a fixture comparison is conformance's, evidence from upstream port readiness's.
  assert.match(python, new RegExp([
    "^\\| Conformance: Not declared: neither `conformance\\.json` nor `origins\\.json` says where it comes from \\|  \\|  \\| 1 \\|",
    "\\| Conformance: Flags nothing in its test fixtures \\|  \\| 1 \\|  \\|",
    "\\| Conformance: Differs from the recorded answers on upstream's test files \\|  \\| 1 \\|  \\|",
    "\\| Conformance: Has no test fixtures \\|  \\| 1 \\|  \\|",
    "\\| Port readiness: Hardcodes names that only appear in upstream's tests \\| 1 \\|  \\|  \\|",
    "\\| Port readiness: Never reproduces one of upstream's findings \\|  \\| 1 \\|  \\|$",
  ].join("\\n"), "mu"));
  // A port no entry maps fails conformance for having no fixtures, so no port readiness step names it.
  assert.doesNotMatch(python, /Not judged against upstream/u);
  // Every candidate of complexity is ready: no failure table.
  assert.doesNotMatch(block(section, "complexity: 1"), /Fails because/u);
  assert.doesNotMatch(text, /How to fix/u);
  // The comment names no failing candidate; the job summary does.
  assert.doesNotMatch(text, /Each failing/u);
});

test("the job summary is the comment's page in full, with each failing candidate's gate and why", () => {
  const text = renderJobSummary(summary(sample()));
  assert.ok(!text.startsWith(COMMENT_MARKER));
  const python = block(text, "Each failing python candidate (6)");
  assert.match(python, /^\*\*Port readiness: Hardcodes names that only appear in upstream's tests\*\*\n\n- `bandit-c` \(python, from bandit BANDIT-C\): 1 upstream test file differs x{400}$/mu);
  assert.match(python, /^- `ruff-g` \(python, from ruff RUFF-G\): /mu);
  assert.match(python, /^- `python-h` \(python, not declared\): /mu);
});

test("thousands of rules still fit in one comment", () => {
  const rules = [
    ...Array.from({ length: 3000 }, (_, index) => verdict(`ruff-shipped-${index}`, "shipped", index % 2 === 0 ? "compact" : null)),
    ...Array.from({ length: 3000 }, (_, index) => verdict(`ruff-candidate-${index}`, "candidate", index % 3 === 0 ? null : "not-overfit")),
  ];
  const text = renderComment(summary(rules));
  assert.ok(text.length <= COMMENT_LIMIT, `${text.length} characters`);
  assert.match(text, /more: see the job summary/u);
  assert.match(text, /^### ❌ Shipped Rule Summary: 1500\/3000 pass, 1500 blocking$/mu);
});

test("a pull request that re-ran only some upstream linters says which it left out", () => {
  const text = renderComment({ ...summary(sample()), notJudged: ["python/bandit", "javascript/eslint"] });
  assert.match(text, /^Not re-run on this change, since nothing their ports read changed: python\/bandit, javascript\/eslint\. Main's results stand for them, and the counts leave them out\.$/mu);
  assert.doesNotMatch(renderComment(summary(sample())), /Not re-run/u);
});

test("a failure is named by the check that failed, in plain words with its fix; an unnamed check falls back to its step", () => {
  assert.deepEqual(failurePoint("in-scope", "in-scope"), { what: "Doesn't check every file type upstream checks", fix: "widen its scope, or narrow `rerail.in_scope` in `gates.yaml`" });
  assert.equal(failureLabel("fixtures", "no-unexplained-false-positives"), "Conformance: Flags code its test fixtures say is fine");
  assert.equal(failureLabel("fixtures", "has-fixtures"), "Conformance: Has no test fixtures");
  assert.equal(failureLabel("fixtures", "identical-on-goldens"), "Conformance: Differs from the recorded answers on real-code goldens");
  assert.equal(failureLabel("upstream-evidence", "stays-silent"), "Port readiness: Never stays silent where upstream does");
  assert.equal(failureLabel("licence", "license-allowed"), "Port readiness: Upstream's licence isn't on Rerail's licence allowlist");
  assert.deepEqual(failurePoint("not-overfit", "a-new-check"), { what: "Not overfit to upstream's tests (a-new-check)", fix: "see the detail" });
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

test("a tool-pack this run recorded gets its per-rule summary in the comment, between the shipped and candidate summaries", () => {
  const markdown = [
    "## Port readiness recordings: 1 tool-pack moved",
    "",
    "Upstream's recorded answers are not in the diff.",
    "",
    "### `python/ruff`",
    "",
    "| Upstream rule | | Expected findings | Corpus files | Facts |",
    "| --- | --- | --- | --- | --- |",
    "| `E501` | changed | +2 / −0 | — | — |",
    "",
  ].join("\n");
  const text = renderComment(summary(sample()), { recordings: [{ toolPack: "python/ruff", markdown }] });
  const start = text.indexOf("### Recorded by this run");
  assert.ok(text.indexOf("### ❌ Shipped Rule Summary") < start && start < text.indexOf("### Candidate Rule Summary"));
  const section = text.slice(start, text.indexOf("### Candidate Rule Summary"));
  assert.match(section, /<summary>python\/ruff: 1 tool-pack moved<\/summary>/u);
  assert.match(section, /^\| `E501` \| changed \| \+2 \/ −0 \| — \| — \|$/mu);
  assert.doesNotMatch(section, /^## Port readiness recordings/mu);
  assert.doesNotMatch(renderComment(summary(sample())), /Recorded by this run/u);
});

test("recording summaries are cut to fit, and the tool-packs past the budget are named", () => {
  const long = `## Port readiness recordings: 1 tool-pack moved\n\n${"| `X` | changed | +1 / −0 | — | — |\n".repeat(1000)}`;
  const recordings = ["a/one", "b/two", "c/three", "d/four"].map((toolPack) => ({ toolPack, markdown: long }));
  const text = renderComment(summary(sample()), { recordings });
  assert.ok(text.length <= COMMENT_LIMIT, `${text.length} characters`);
  assert.match(text, /Cut short: the leg's job summary has all of it\./u);
  assert.match(text, /^Also recorded, too long to show here \(see each leg's job summary\): c\/three, d\/four\.$/mu);
});
