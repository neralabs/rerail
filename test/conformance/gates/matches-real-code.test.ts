import assert from "node:assert/strict";
import { test } from "vitest";

import { patchedPaths, type CorpusFile } from "../../../conformance/gates/corpus.js";
import type { CompiledPolicy, Engine } from "../../../conformance/gates/engine.js";
import { CASES_PER_KIND, constructWords, differingFiles, matchesRealCodeGate } from "../../../conformance/gates/checks/matches-real-code.js";
import { renderMarkdown } from "../../../conformance/gates/report.js";
import { summarizeRules, type GatesReport, type RuleReport } from "../../../conformance/gates/run.js";
import type { PackAdapter, Span, UpstreamFile } from "../../../conformance/gates/types.js";

// Sources are single lines of ASCII, so a byte offset is the column.
const span = (start: number, end: number): Span => ({ startLine: 0, startCol: start, endLine: 0, endCol: end });

const engine = { rerail: { dialectForPath: (path: string) => (path.endsWith(".ts") ? "typescript" : undefined) } } as unknown as Engine;

const adapter = {
  keywords: new Set(["debugger", "if"]),
  toSpan: (_name: string, _source: string, startByte: number, endByte: number) => span(startByte, endByte),
} as unknown as PackAdapter;

const DOCUMENT = { clauses: [{ query: { kind: "debugger_statement" } }] };

/** A rule that reports the given byte ranges in each file and fails to parse the files listed as unparsed. */
function compiled(results: Record<string, [number, number][]>, inScope: (path: string) => boolean = () => true, unparsed: string[] = []): CompiledPolicy {
  return {
    canonicalJson: "{}",
    digest: "sha256:0",
    inScope,
    evaluateFile(path) {
      return {
        status: "completed",
        failures: unparsed.includes(path) ? [{ code: "source-incomplete", message: "query source is incomplete" }] : [],
        violations: (results[path] ?? []).map(([startByte, endByte]) => ({ clauseId: "c", code: "C", path, span: { startByte, endByte } })),
      };
    },
  };
}

function file(task: string, path: string, expected: Span[], source = "const x = 1;", excluded: string | null = null): UpstreamFile {
  const corpusFile: CorpusFile = { task, repository: "o/r", path, source, sha256: "0".repeat(64) };
  return { file: corpusFile, excluded, findings: new Map([["no-debugger", expected]]) };
}

test("patchedPaths reads modified, created, deleted and renamed files", () => {
  const patch = [
    "diff --git a/src/a.ts b/src/a.ts",
    "index 1..2 100644",
    "--- a/src/a.ts",
    "+++ b/src/a.ts",
    "@@ -1 +1 @@",
    "diff --git a/src/new.ts b/src/new.ts",
    "new file mode 100644",
    "--- /dev/null",
    "+++ b/src/new.ts",
    "diff --git a/src/gone.ts b/src/gone.ts",
    "deleted file mode 100644",
    "--- a/src/gone.ts",
    "+++ /dev/null",
    "diff --git a/src/old.ts b/src/moved.ts",
    "similarity index 100%",
    "rename from src/old.ts",
    "rename to src/moved.ts",
  ].join("\n");
  assert.deepEqual(patchedPaths(patch), [
    { before: "src/a.ts", after: "src/a.ts" },
    { before: null, after: "src/new.ts" },
    { before: "src/gone.ts", after: null },
    { before: "src/old.ts", after: "src/moved.ts" },
  ]);
});

test("constructWords names a rule's constructs from its node kinds", () => {
  assert.deepEqual([...constructWords(DOCUMENT, adapter.keywords)], ["debugger"]);
});

test("a rule that agrees with upstream on every case passes, and cases come from upstream's verdicts alone", () => {
  const files = [
    file("t2", "src/b.ts", [span(0, 9)]),
    file("t1", "src/z.ts", [span(0, 9)]),
    file("t1", "src/plain.ts", []),
    file("t1", "src/uses.ts", [], "if (debugger) {}"),
  ];
  const result = matchesRealCodeGate(engine, adapter, compiled({ "src/b.ts": [[0, 9]], "src/z.ts": [[0, 9]] }), "no-debugger", DOCUMENT, files);
  assert.equal(result.checks[0]!.ok, true);
  assert.equal(result.positives, 2);
  assert.deepEqual(result.cases.map((testCase) => [testCase.kind, testCase.task, testCase.path, testCase.status]), [
    ["positive", "t1", "src/z.ts", "agree"],
    ["positive", "t2", "src/b.ts", "agree"],
    // The negative that contains the rule's own construct comes first.
    ["negative", "t1", "src/uses.ts", "agree"],
    ["negative", "t1", "src/plain.ts", "agree"],
  ]);
});

test("agreement is required in both directions: a missing or an extra finding fails the gate", () => {
  const files = [file("t1", "src/a.ts", [span(0, 9)]), file("t1", "src/b.ts", [])];
  const missing = matchesRealCodeGate(engine, adapter, compiled({}), "no-debugger", DOCUMENT, files);
  assert.equal(missing.checks[0]!.ok, false);
  assert.deepEqual(missing.cases[0]!.missing, [span(0, 9)]);
  assert.match(missing.checks[0]!.detail, /^1 of 2 case\(s\) differ, e\.g\. positive t1\/src\/a\.ts: upstream 1, rule 0$/u);

  const extra = matchesRealCodeGate(engine, adapter, compiled({ "src/a.ts": [[0, 9]], "src/b.ts": [[2, 4]] }), "no-debugger", DOCUMENT, files);
  assert.equal(extra.checks[0]!.ok, false);
  assert.deepEqual(extra.cases.map((testCase) => testCase.status), ["agree", "mismatch"]);
  assert.deepEqual(extra.cases[1]!.extra, [span(2, 4)]);

  const endsElsewhere = matchesRealCodeGate(engine, adapter, compiled({ "src/a.ts": [[0, 8]] }), "no-debugger", DOCUMENT, files);
  assert.equal(endsElsewhere.checks[0]!.ok, false);
});

test("a rule with no file where upstream reports it agrees on its negatives, and nothing more is said", () => {
  const result = matchesRealCodeGate(engine, adapter, compiled({}), "no-debugger", DOCUMENT, [file("t1", "src/a.ts", [])]);
  assert.equal(result.checks[0]!.ok, true);
  assert.equal(result.positives, 0);
  assert.equal(result.checks[0]!.detail, "agrees with upstream on all 1 case(s) (0 positive) from 1 eligible file(s)");
});

test("a rule with no eligible file is judged on nothing and reported not-run, not as agreeing", () => {
  const outOfScope = [file("t1", "src/a.mts", [span(0, 9)]), file("t1", "src/b.ts", [], "const x = 1;", "TS1005 ';' expected.")];
  for (const files of [[], outOfScope]) {
    const result = matchesRealCodeGate(engine, adapter, compiled({}), "no-debugger", DOCUMENT, files);
    assert.equal(result.eligible, 0);
    assert.deepEqual(result.cases, []);
    assert.deepEqual(result.checks, [{
      id: "not-run",
      ok: false,
      detail: `0 eligible files: none of the corpus's ${files.length} file(s) is one the document applies to and upstream reads, so nothing was judged`,
    }]);
  }
});

test("the report names the rules matches-real-code judged on no file instead of calling them in agreement", () => {
  const rule = (id: string, eligible: number): RuleReport => {
    const ok = { pass: true, checks: [] };
    return {
      id, file: `packs/demo/rules/${id}.json`, location: "shipped", upstreamRule: id, sha256: "0", bytes: 1, policyDigest: null, pass: true, stoppedAt: null,
      blockedBy: null, defaultOn: true, defaultOnReason: "", on: true,
      gates: {
        "upstream-status": { checks: [] }, loads: ok, "not-overfit": { ...ok, flagged: [], words: {}, uncheckedRegexes: [] }, compact: { ...ok, metrics: null },
        "matches-upstream-tests": { ...ok, totals: null, cases: {}, differences: [], goldens: { count: 0, differing: { count: 0, first: [] }, engineGaps: { count: 0, first: [] } } },
        "matches-real-code": { checks: [], eligible, positives: 0, cases: [], disagreementsElsewhere: { count: 0, first: [] } },
        "passes-conformance": ok,
      },
    };
  };
  const report = (rules: RuleReport[]): GatesReport => ({
    format: "rerail.rule-gates/v1", pack: "demo", tool: "tool", toolPack: "demo/tool",
    upstream: { name: "", repository: "", commit: "", version: "", license: "", recommended_configs: [], runtime: "", identity: {} },
    engine: { sha256: "e" }, gates: [], method: {} as GatesReport["method"], toolPackChecks: { pass: true, checks: [] }, integrity: [], knownGaps: [], gapCandidates: [],
    corpus: { tasks: [], files: 0, excluded: 0 }, unmapped: [], listedWithoutDocument: 0, cache: { rules: rules.length, hits: 0 }, uncommittedPromotions: 0,
    summary: summarizeRules(rules), rules,
  });

  const none = renderMarkdown(report([rule("a", 0), rule("b", 0)]));
  assert.match(none, /^2 of 2 rule\(s\) were judged on no file \(0 eligible files, so matches-real-code is not-run and says nothing about them\): `a`, `b`\.$/mu);
  assert.match(none, /^No rule was judged on any file\.$/mu);
  assert.doesNotMatch(none, /No rule differs from upstream/u);

  const some = renderMarkdown(report([rule("a", 0), rule("b", 3)]));
  assert.match(some, /^1 of 2 rule\(s\) were judged on no file .*: `a`\.$/mu);
  assert.match(some, /^No rule differs from upstream on any eligible file\.$/mu);

  assert.doesNotMatch(renderMarkdown(report([rule("b", 3)])), /judged on no file/u);
});

test("only files the document applies to and upstream can read are eligible", () => {
  const files = [
    file("t1", "src/a.ts", [span(0, 9)]),
    file("t1", "src/broken.ts", [span(0, 9)], "const x = 1;", "TS1005 ';' expected."),
    file("t1", "src/vendor.ts", [span(0, 9)]),
    file("t1", "src/a.mts", [span(0, 9)]),
  ];
  const result = matchesRealCodeGate(engine, adapter, compiled({ "src/a.ts": [[0, 9]] }, (path) => !path.includes("vendor")), "no-debugger", DOCUMENT, files);
  assert.equal(result.eligible, 1);
  assert.deepEqual(result.cases.map((testCase) => testCase.path), ["src/a.ts"]);
});

test("cases are capped per kind, and disagreements outside them are reported without failing the gate", () => {
  const positives = Array.from({ length: CASES_PER_KIND + 1 }, (_, index) => file("t1", `src/p${index}.ts`, [span(0, 9)]));
  const agreeing = Object.fromEntries(positives.slice(0, CASES_PER_KIND).map((entry) => [entry.file.path, [[0, 9]] as [number, number][]]));
  const result = matchesRealCodeGate(engine, adapter, compiled(agreeing), "no-debugger", DOCUMENT, positives);
  assert.equal(result.cases.length, CASES_PER_KIND);
  assert.equal(result.positives, CASES_PER_KIND + 1);
  assert.equal(result.checks[0]!.ok, true);
  assert.deepEqual(result.disagreementsElsewhere, [{ task: "t1", path: `src/p${CASES_PER_KIND}.ts`, expected: 1, actual: 0 }]);
});

test("an engine parse failure on an eligible file fails the case and is handed on for independent parsers", () => {
  const files = [file("t1", "src/a.ts", [span(0, 9)]), file("t1", "src/styled.ts", [])];
  const result = matchesRealCodeGate(engine, adapter, compiled({ "src/a.ts": [[0, 9]] }, () => true, ["src/styled.ts"]), "no-debugger", DOCUMENT, files);
  assert.equal(result.checks[0]!.ok, false);
  assert.deepEqual(result.cases[1]!.failures, ["source-incomplete: query source is incomplete"]);
  assert.deepEqual(result.unparsed, [{ gate: "matches-real-code", file: "t1/src/styled.ts", path: "src/styled.ts", source: "const x = 1;", script: false, engine: "source-incomplete: query source is incomplete" }]);
});

test("every eligible file where the rule and upstream differ becomes a golden, chosen as a case or not, in task and path order", () => {
  const positives = Array.from({ length: CASES_PER_KIND + 2 }, (_, index) => file(index % 2 === 0 ? "t2" : "t1", `src/p${index}.ts`, [span(0, 9)]));
  // The rule misses the first chosen case and two files beyond the cases matches-real-code chooses.
  const reported = Object.fromEntries(positives.slice(1, CASES_PER_KIND).map((entry) => [entry.file.path, [[0, 9]] as [number, number][]]));
  const result = matchesRealCodeGate(engine, adapter, compiled(reported), "no-debugger", DOCUMENT, positives);
  const chosen = result.cases.filter((testCase) => testCase.status === "mismatch").map((testCase) => `${testCase.task}/${testCase.path}`);
  const elsewhere = result.disagreementsElsewhere.map((entry) => `${entry.task}/${entry.path}`);
  assert.ok(chosen.length > 0 && elsewhere.length > 0);
  const differing = differingFiles(result).map((entry) => `${entry.task}/${entry.path}`);
  assert.deepEqual(differing, [...chosen, ...elsewhere].sort());
  // A file where the rule agrees never becomes a golden.
  assert.equal(differing.length, chosen.length + elsewhere.length);
  assert.equal(differingFiles({ cases: [], disagreementsElsewhere: [] }).length, 0);
});
