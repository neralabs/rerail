import assert from "node:assert/strict";
import { test } from "vitest";

import type { CompiledPolicy, Engine, EngineEvaluation } from "../../../conformance/port-readiness/engine.js";
import { classifyParsers, provenGaps, matchesUpstreamTestsGate, outsideInScope, type ProvenGap } from "../../../conformance/port-readiness/checks/matches-upstream-tests.js";
import type { KnownGap, PackAdapter, ParserVerdict, Span, UpstreamCase } from "../../../conformance/port-readiness/types.js";

// Sources are single lines of ASCII, so a byte offset is the column.
const span = (start: number, end: number): Span => ({ startLine: 0, startCol: start, endLine: 0, endCol: end });

const engine = { rerail: { dialectForPath: (path: string) => (path.endsWith(".ts") ? "typescript" : undefined) } } as unknown as Engine;

const adapter = {
  toSpan: (_name: string, _source: string, startByte: number, endByte: number) => span(startByte, endByte),
} as unknown as PackAdapter;

function compiled(results: Record<string, Partial<EngineEvaluation> & { spans?: [number, number][] }>): CompiledPolicy {
  return {
    canonicalJson: "{}",
    digest: "sha256:0",
    inScope: (path) => path.endsWith(".ts"),
    evaluateFile(path) {
      const result = results[path] ?? {};
      return {
        status: "completed",
        failures: result.failures ?? [],
        violations: (result.spans ?? []).map(([startByte, endByte]) => ({ clauseId: "c", code: "C", path, span: { startByte, endByte } })),
      };
    },
  };
}

function upstreamCase(path: string, expected: Span[], overrides: Partial<UpstreamCase> = {}): UpstreamCase {
  return { fixture: `test/rules/${path}.lint`, variant: ".", path, source: "x".repeat(40), skipped: null, expected, markup: expected, ...overrides };
}

const partial = { code: "source-partial", message: "file has syntax errors" };
const incomplete = { code: "source-incomplete", message: "query source is incomplete" };

test("identical spans pass and count as upstream evidence", () => {
  const result = matchesUpstreamTestsGate(engine, adapter, "r", compiled({ "r/a.ts": { spans: [[2, 5], [7, 9]] } }), [upstreamCase("r/a.ts", [span(7, 9), span(2, 5)])], new Map());
  assert.deepEqual(result.checks.map((check) => check.ok), [true, true]);
  assert.deepEqual(result.totals, { scoredCases: 1, expected: 2, matched: 2, missing: 0, extra: 0, startsMatched: 2 });
});

test("a finding that ends elsewhere is a mismatch even when every start agrees", () => {
  const result = matchesUpstreamTestsGate(engine, adapter, "r", compiled({ "r/a.ts": { spans: [[2, 6]] } }), [upstreamCase("r/a.ts", [span(2, 5)])], new Map());
  assert.equal(result.cases[0]!.status, "mismatch");
  assert.equal(result.cases[0]!.startsAgree, true);
  assert.equal(result.totals.startsMatched, 1);
  assert.deepEqual(result.checks.map((check) => check.ok), [false, false]);
});

test("findings compare as multisets", () => {
  const result = matchesUpstreamTestsGate(engine, adapter, "r", compiled({ "r/a.ts": { spans: [[2, 5]] } }), [upstreamCase("r/a.ts", [span(2, 5), span(2, 5)])], new Map());
  assert.equal(result.cases[0]!.missing.length, 1);
  assert.equal(result.cases[0]!.status, "mismatch");
});

test("a rule compared without an end says so", () => {
  const full = matchesUpstreamTestsGate(engine, adapter, "r", compiled({ "r/a.ts": { spans: [[2, 5]] } }), [upstreamCase("r/a.ts", [span(2, 5)])], new Map());
  assert.deepEqual(full.compared, { endLine: true, endCol: true });
  assert.equal(full.checks[0]!.detail, "identical on 1 scored file(s)");

  // An ESLint rule some of whose findings on upstream's tests have no end: toSpan keeps only the start.
  const startOnly = { toSpan: (_name: string, _source: string, startByte: number) => ({ startLine: 0, startCol: startByte, endLine: null, endCol: null }) } as unknown as PackAdapter;
  const start = (column: number): Span => ({ startLine: 0, startCol: column, endLine: null, endCol: null });
  const result = matchesUpstreamTestsGate(engine, startOnly, "r", compiled({ "r/a.ts": { spans: [[2, 6]] }, "r/b.ts": { spans: [[3, 4]] } }),
    [upstreamCase("r/a.ts", [start(2)]), upstreamCase("r/b.ts", [start(1)])], new Map());
  assert.deepEqual(result.compared, { endLine: false, endCol: false });
  assert.equal(result.checks[0]!.detail, "1 upstream test file(s) differ, e.g. test/rules/r/b.ts.lint: missing 1:2, extra 1:4; compared on start only for every finding, since not every upstream finding of the rule carries the rest");
});

test("an engine parse failure is a mismatch, never excused on its own, and is handed on for independent parsers", () => {
  const result = matchesUpstreamTestsGate(engine, adapter, "r", compiled({ "r/a.ts": { failures: [incomplete] } }), [upstreamCase("r/a.ts", [span(1, 2)])], new Map());
  assert.equal(result.cases[0]!.status, "mismatch");
  assert.deepEqual(result.checks.map((check) => check.ok), [false, false]);
  assert.deepEqual(result.unparsed, [{ gate: "matches-upstream-tests", file: "test/rules/r/a.ts.lint", path: "r/a.ts", source: "x".repeat(40), script: false, engine: "source-incomplete: query source is incomplete" }]);

  const parsed = matchesUpstreamTestsGate(engine, adapter, "r", compiled({ "r/a.ts": { spans: [[1, 3]] } }), [upstreamCase("r/a.ts", [span(1, 2)])], new Map());
  assert.equal(parsed.cases[0]!.status, "mismatch");
  assert.deepEqual(parsed.unparsed, []);
});

test("a proven known gap excuses only the fixtures it lists, and only on a parse failure", () => {
  const gap: KnownGap = { id: "paren-target", probe: "[(a)] = [];", fixtures: ["test/rules/r/a.ts.lint"], reason: "tree-sitter" };
  const gaps = new Map<string, ProvenGap>([[gap.id, { gap, probePath: "probe.ts" }]]);
  const result = matchesUpstreamTestsGate(engine, adapter, "r", compiled({
    "r/a.ts": { failures: [partial] },
    "r/b.ts": { failures: [partial] },
    "r/c.ts": { spans: [[1, 2]] },
  }), [upstreamCase("r/a.ts", [span(1, 2)]), upstreamCase("r/b.ts", [span(1, 2)]), upstreamCase("r/c.ts", [span(1, 2)])], gaps);
  assert.deepEqual(result.cases.map((testCase) => testCase.status), ["excused", "mismatch", "agree"]);
  assert.deepEqual(result.gapUses, { "paren-target": 1 });
  assert.deepEqual(result.unparsed.map((file) => file.file), ["test/rules/r/b.ts.lint"]);
  assert.match(result.checks[0]!.detail, /^1 upstream test file\(s\) differ, e\.g\. test\/rules\/r\/b\.ts\.lint/u);

  const listedOnly = matchesUpstreamTestsGate(engine, adapter, "r", compiled({ "r/a.ts": { failures: [partial] }, "r/c.ts": { spans: [[1, 2]] } }),
    [upstreamCase("r/a.ts", [span(1, 2)]), upstreamCase("r/c.ts", [span(1, 2)])], gaps);
  assert.equal(listedOnly.checks[0]!.detail, "identical on 1 scored file(s); 1 file(s) excused by listed known gaps");
});

test("skipped files, and unreachable files where upstream reports nothing, are not scored; markup disagreements are reported", () => {
  const result = matchesUpstreamTestsGate(engine, adapter, "r", compiled({ "r/d.ts": { spans: [[1, 2]] } }), [
    upstreamCase("r/a.ts", [], { skipped: "requires TypeScript <2.7" }),
    upstreamCase("r/b.js", []),
    upstreamCase("r/c", []),
    upstreamCase("r/d.ts", [span(1, 2)], { markup: [span(1, 3)] }),
  ], new Map());
  assert.deepEqual(result.cases.map((testCase) => testCase.status), ["not-scored", "not-scored", "not-scored", "agree"]);
  assert.deepEqual(result.markupDisagreements, ["test/rules/r/d.ts.lint: upstream's output differs from its own test markup"]);
});

test("upstream findings in a file the rule cannot reach are a difference, not a skipped file", () => {
  const result = matchesUpstreamTestsGate(engine, adapter, "r", compiled({ "r/d.ts": { spans: [[1, 2]] } }), [
    upstreamCase("r/types.js", [span(1, 2), span(3, 4)]),
    upstreamCase("r/README", [span(1, 2)]),
    upstreamCase("r/d.ts", [span(1, 2)]),
  ], new Map());
  assert.deepEqual(result.cases.map((testCase) => testCase.status), ["mismatch", "mismatch", "agree"]);
  const scoped = { ...compiled({}), inScope: (path: string) => !path.includes("excluded") };
  const excluded = matchesUpstreamTestsGate(engine, adapter, "r", scoped, [upstreamCase("r/excluded.ts", [span(1, 2)])], new Map());
  assert.equal(excluded.cases[0]!.reason, "the file is outside the shipped document's scope, but upstream reports 1 finding(s) in it");
  assert.equal(result.cases[0]!.reason, "RERAIL has no dialect for .js, but upstream reports 2 finding(s) in it");
  assert.equal(result.cases[1]!.reason, "RERAIL has no dialect for files without an extension, but upstream reports 1 finding(s) in it");
  assert.equal(result.totals.missing, 3);
  assert.equal(result.checks[0]!.ok, false);
});

test("a case of a file type outside the tool-pack's rerail.in_scope is not scored, and is counted apart", () => {
  const narrowed = { ...adapter, inScopeTypes: [".ts"] } as PackAdapter;
  const cases = [
    upstreamCase("r/a.js", [span(1, 2)]),
    upstreamCase("r/b.JS", []),
    // A script case at a .vue name is read in the type its parser read it in.
    upstreamCase("r/c.ts", [span(1, 2)], { parsedAs: ".js" }),
    upstreamCase("r/d.ts", [span(1, 2)]),
    upstreamCase("r/e.ts", []),
  ];
  const result = matchesUpstreamTestsGate(engine, narrowed, "r", compiled({ "r/d.ts": { spans: [[1, 2]] } }), cases, new Map());
  assert.deepEqual(result.cases.map((testCase) => testCase.status), ["not-scored", "not-scored", "not-scored", "agree", "agree"]);
  assert.deepEqual(result.cases.map((testCase) => testCase.reason).slice(0, 3), [
    "outside the tool-pack's rerail.in_scope (.js)",
    "outside the tool-pack's rerail.in_scope (.js)",
    "outside the tool-pack's rerail.in_scope (.js)",
  ]);
  assert.deepEqual(result.cases.map(outsideInScope), [true, true, true, false, false]);
  assert.deepEqual(result.totals, { scoredCases: 2, expected: 1, matched: 1, missing: 0, extra: 0, startsMatched: 1 });
  assert.deepEqual(result.checks.map((check) => check.ok), [true, true]);
  assert.equal(result.checks[0]!.detail, "identical on 2 scored file(s); 3 case(s) outside the tool-pack's rerail.in_scope not scored");

  // fires counts in-scope cases alone: out-of-scope agreement is no evidence.
  const outOnly = matchesUpstreamTestsGate(engine, narrowed, "r", compiled({}), cases.slice(0, 3), new Map());
  assert.equal(outOnly.checks[1]!.ok, false);

  // Without in_scope, the same .js case is the difference it always was.
  const unnarrowed = matchesUpstreamTestsGate(engine, adapter, "r", compiled({}), cases.slice(0, 1), new Map());
  assert.equal(unnarrowed.cases[0]!.status, "mismatch");
});

// This engine fails to parse anything as .ts and parses everything as .tsx.
const probing = {
  rerail: { dialectForPath: (path: string) => (path.endsWith(".tsx") ? "tsx" : path.endsWith(".ts") ? "typescript" : undefined) },
  compile: () => ({
    evaluateFile: (_path: string, language: string) => ({ status: "failed", violations: [], failures: language === "typescript" ? [partial] : [] }),
  }),
} as unknown as Engine;
const parsersAccept = async (files: unknown[]) => files.map(() => [{ parser: "ts", syntax: [], grammar: [] }, { parser: "babel", syntax: [], grammar: [] }]);

test("a known gap is proven under the file type of the fixtures it names", async () => {
  const gap = (fixtures: string[]): KnownGap => ({ id: "jsx", probe: "<a b='c' />", fixtures, reason: "tree-sitter" });
  const source = "const a = <a b='c' />;";
  const cases = [{ fixture: "test/rules/q/x.ts.lint", path: "q/x.ts", source }, { fixture: "test/rules/q/x.tsx.lint", path: "q/x.tsx", source }];
  const proven = await provenGaps(probing, [
    gap(["test/rules/q/x.ts.lint"]),
    { ...gap(["test/rules/q/x.tsx.lint"]), id: "tsx" },
    { ...gap(["test/rules/q/x.ts.lint", "test/rules/q/x.tsx.lint"]), id: "mixed" },
    { ...gap(["test/rules/q/elsewhere.ts.lint"]), id: "stray" },
  ], cases, parsersAccept);
  assert.deepEqual([...proven.proven.keys()], ["jsx"]);
  assert.equal(proven.proven.get("jsx")!.probePath, "probe.ts");
  assert.deepEqual(proven.problems, [
    "known gap tsx: the engine parses the probe as .tsx, so the gap is closed",
    "known gap mixed: its fixtures must share one file type, found .ts, .tsx",
    "known gap stray: test/rules/q/elsewhere.ts.lint is not an upstream test file of the pack",
  ]);
});

test("a known gap holds only when its probe is in every file it names and independent parsers read the probe as valid source", async () => {
  const gap = (id: string, probe: string, fixtures = ["test/rules/q/a.ts.lint"]): KnownGap => ({ id, probe, fixtures, reason: "tree-sitter" });
  const cases = [
    { fixture: "test/rules/q/a.ts.lint", path: "q/a.ts", source: "[(a)] = []; f(x)));", script: true },
    { fixture: "test/rules/q/b.ts.lint", path: "q/b.ts", source: "let b = 1;", script: true },
  ];
  const asked: { path: string; source: string; script?: boolean }[] = [];
  const verdicts: Record<string, ParserVerdict[]> = {
    "[(a)] = [];": [{ parser: "ts", syntax: [], grammar: [] }, { parser: "babel", syntax: [], grammar: [] }],
    ")))": [{ parser: "ts", syntax: ["TS1128 Declaration or statement expected."], grammar: [] }, { parser: "babel", syntax: ["Unexpected token"], grammar: [] }],
    "f(x)": [{ parser: "ts", syntax: [], grammar: [] }, { parser: "babel", syntax: ["Unexpected token"], grammar: [] }],
  };
  const proven = await provenGaps(probing, [
    gap("paren-target", "[(a)] = [];"),
    gap("elsewhere", "[(a)] = [];", ["test/rules/q/a.ts.lint", "test/rules/q/b.ts.lint"]),
    gap("not-source", ")))"),
    gap("disputed", "f(x)"),
    gap("unrecorded", "f(x)));"),
  ], cases, async (files) => files.map((file) => {
    asked.push(file);
    return verdicts[file.source] ?? [];
  }));
  assert.deepEqual([...proven.proven.keys()], ["paren-target"]);
  assert.deepEqual(proven.problems, [
    "known gap elsewhere: the probe does not occur in test/rules/q/b.ts.lint",
    "known gap not-source: independent parsers do not read the probe as valid source (invalid-source), so the parse failure is not the engine's",
    "known gap disputed: independent parsers do not read the probe as valid source (parsers-disagree), so the parse failure is not the engine's",
    "known gap unrecorded: no independent parser verdict on the probe is recorded; record the tool-pack again",
  ]);
  assert.deepEqual(Object.fromEntries(proven.kinds), { "paren-target": "valid-source", "not-source": "invalid-source", disputed: "parsers-disagree", unrecorded: "not-recorded" });
  // The probe is read as the gates recorder read it: under the fixtures' file type, as a script when they all are.
  assert.deepEqual(asked[0], { path: "probe.ts", source: "[(a)] = [];", script: true });
});

test("a file is invalid source only when every independent parser reports a syntax or grammar error", () => {
  const verdict = (parser: string, syntax: string[] = [], grammar: string[] = []): ParserVerdict => ({ parser, syntax, grammar });
  assert.equal(classifyParsers([verdict("ts", ["TS1005 ';' expected."]), verdict("babel", ["Unexpected token"])]), "invalid-source");
  // A tree every parser built, with what the language forbids in it, is still not valid source.
  assert.equal(classifyParsers([verdict("ts", [], ["TS1097 'extends' list cannot be empty."]), verdict("babel", [], ["'extends' list cannot be empty."])]), "invalid-source");
  assert.equal(classifyParsers([verdict("ts"), verdict("babel")]), "valid-source");
  assert.equal(classifyParsers([verdict("ts", ["TS1005 ';' expected."]), verdict("babel")]), "parsers-disagree");
  assert.equal(classifyParsers([verdict("ts", [], ["TS1028 Accessibility modifier already seen."]), verdict("babel")]), "parsers-disagree");
});

test("a parser that makes no grammar checks has no say when every error is a grammar error", () => {
  const cpython = (syntax: string[], grammar: string[]): ParserVerdict => ({ parser: "cpython", syntax, grammar });
  const libcst = (syntax: string[] = []): ParserVerdict => ({ parser: "libcst", syntax, grammar: null });
  const outside = ["SyntaxError: 'return' outside function"];
  assert.equal(classifyParsers([cpython([], outside), libcst(), { parser: "parso", syntax: [], grammar: outside }]), "invalid-source");
  assert.equal(classifyParsers([cpython([], []), libcst()]), "valid-source");
  // A syntax error is everyone's business: libcst reading the file is a real disagreement.
  assert.equal(classifyParsers([cpython(["SyntaxError: invalid syntax"], []), libcst()]), "parsers-disagree");
  assert.equal(classifyParsers([cpython(["SyntaxError: invalid syntax"], []), libcst(["Syntax Error @ 1:1"])]), "invalid-source");
});

test("a case upstream's parser read in another language than its path's is evaluated at its path, in that language", () => {
  const seen: [string, string][] = [];
  const dialects = { rerail: { dialectForPath: (path: string) => (path.endsWith(".jsx") ? "javascript" : path.endsWith(".ts") ? "typescript" : undefined) } } as unknown as Engine;
  const recording: CompiledPolicy = {
    canonicalJson: "{}",
    digest: "sha256:0",
    inScope: () => true,
    evaluateFile(path, language) {
      seen.push([path, language]);
      return { status: "completed", violations: [], failures: [] };
    },
  };
  matchesUpstreamTestsGate(dialects, adapter, "r", recording, [upstreamCase("pages/index.ts", [], { parsedAs: ".jsx" }), upstreamCase("src/a.ts", [])], new Map());
  assert.deepEqual(seen, [["pages/index.ts", "javascript"], ["src/a.ts", "typescript"]]);
});
