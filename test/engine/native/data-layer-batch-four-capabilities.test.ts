import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { loadNativeBinding } from "../../../src/engine/native/native.js";

const native = loadNativeBinding();
const capabilities = readFileSync(new URL("../../../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8");
const compile = (clause: object, limits: Record<string, number> = {}) => native.CodeRuleDslRuntime.compile(JSON.stringify({
  format: "rerail.policy/v1", id: "fourth-batch-capability", revision: 1, mode: "steer",
  scope: { include: ["**/*.py"], exclude: [] }, clauses: [{ id: "check",
    violation: { code: "CHECK", anchor: "witness.span", message: "check" }, ...clause }],
}), "json", JSON.stringify({ ...JSON.parse(capabilities), limits: { ...JSON.parse(capabilities).limits, ...limits } }));
const evaluate = (rule: ReturnType<typeof compile>, source: string) => JSON.parse(rule.evaluate(JSON.stringify({
  format: "rerail.repository-snapshot/v1", revision: 0, files: [{ path: "a.py", source, language: "python" }], facts: [],
}), "report"));

it("joins every decoded identifier word to the host, including later container arguments", () => {
  const rule = compile({ forbid: { ast: { language: "python", rule: { kind: "string", parse: {
    grammar: "identifierWords", has: { kind: "word", capture: "WORD", host: { inside: {
      kind: "module", has: { kind: "aliased_import", has: { field: "alias", pattern: "$ALIAS" },
        compare: { left: "WORD", right: "ALIAS", op: "eq" }, stopBy: "end" }, stopBy: "end",
    } } },
  } } } } });
  const result = evaluate(rule, "from package import SqlFragment as F\na: 'list[str, F]'\nb: 'list[str, Foo]'\nc: 'list[\\x46]'\nd: 'not a type F!'\n");
  expect(result.failures).toEqual([]);
  expect(result.violations.map((v: { span: { startLine: number } }) => v.span.startLine)).toEqual([1, 3, 4]);
});

const closureClause = {
  budget: { query: { evidence: { as: "row", query: { ast: { language: "python", rule: { any: [
    { kind: "assignment", has: { field: "left", pattern: "$TO" }, all: [{ has: { field: "right", pattern: "$FROM" } }] },
    { kind: "call", has: { field: "function", pattern: "$MEMBER" } },
  ] } } }, project: { from: "capture.FROM", to: "capture.TO", member: "capture.MEMBER", path: "file.path" },
  closure: [{ name: "known", seeds: ["Root"], edges: { from: "from", to: "to" }, contains: { known: "member" } }],
  where: { eq: ["row.known", true] } } }, count: { lte: 0 }, group_by: ["row.path", "row.member"] },
};

it("computes cyclic name closure across files and invalidates it after edits and deletions", async () => {
  const rule = compile(closureClause);
  const files = [
    { path: "a.py", language: "python", source: "A = B\nB = A\nB = Root\n" },
    { path: "b.py", language: "python", source: "A()\nOther()\n" },
  ];
  const snapshot = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files, facts: [] }), "report"));
  expect(snapshot.failures).toEqual([]);
  expect(snapshot.violations).toHaveLength(1);
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(rule);
    const apply = async (revision: number, changed: typeof files, removed: string[] = []) => JSON.parse(await workspace.applyAsync(JSON.stringify({ revision, files: changed, removed }))).evaluation;
    expect(await apply(1, files)).toMatchObject({ violations: snapshot.violations, failures: [] });
    expect(await apply(2, [{ ...files[0]!, source: "A = B\nB = A\n" }])).toMatchObject({ violations: [], failures: [] });
    expect(await apply(3, [files[0]!])).toMatchObject({ violations: snapshot.violations, failures: [] });
    expect(await apply(4, [], ["a.py"])).toMatchObject({ violations: [], failures: [] });
  } finally { workspace.close(); }
});

it("keeps closure partitions separate and stages ordered", () => {
  const closure = [
    { name: "classes", seeds: ["Root"], edges: { from: "from", to: "to" }, contains: {} },
    { name: "known", seeds: [], edges: { from: "from", to: "to" }, contains: { known: "member" },
      extends: ["classes"], partition_by: ["path"] },
  ];
  const clause = { budget: { ...closureClause.budget, query: { evidence: { ...closureClause.budget.query.evidence, closure } } } };
  // An earlier set with a different partition would make inheritance ambiguous.
  expect(() => compile(clause)).toThrow("same partition_by");
  expect(() => compile({ budget: { ...closureClause.budget, query: { evidence: { ...closureClause.budget.query.evidence,
    closure: [{ name: "known", seeds: ["Root"], edges: { from: "missing", to: "to" }, contains: {} }],
  } } } })).toThrow("projected field");
  expect(() => compile({ budget: { ...closureClause.budget, query: { evidence: { ...closureClause.budget.query.evidence,
    closure: [{ name: "known", seeds: ["Root"], edges: { from: "from", to: "to" }, contains: { member: "member" } }],
  } } } })).toThrow("must be new");
});

it("fails closure explicitly when a source file cannot provide complete facts", () => {
  const result = evaluate(compile(closureClause), "A = Root\nA()\ndef broken(:\n");
  expect(result.violations).toEqual([]);
  expect(result.failures.length).toBeGreaterThan(0);
});

it("uses calibrated workspace work polls for a large staged closure", async () => {
  const evidence = { ...closureClause.budget.query.evidence, closure: [
    { name: "all", seeds: ["Root"], edges: { from: "from", to: "to" }, contains: {} },
    { name: "known", extends: ["all"], contains: { known: "member" } },
  ] };
  const rule = compile({ budget: { ...closureClause.budget, query: { evidence } } });
  const source = Array.from({ length: 6_000 }, (_, index) => `A${index} = ${index === 0 ? "Root" : `A${index - 1}`}`).join("\n") + "\nA5999()\n";
  const expected = evaluate(rule, source);
  expect(expected.failures).toEqual([]);
  expect(expected.violations).toHaveLength(1);
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(rule);
    const result = JSON.parse(await workspace.applyAsync(JSON.stringify({ revision: 1, files: [{ path: "a.py", source, language: "python" }], removed: [] })));
    expect(result.evaluation.failures).toEqual([]);
    expect(result.evaluation.violations).toEqual(expected.violations);
  } finally { workspace.close(); }
});

it("projects f-string literals and a template without reading interpolation expressions", () => {
  const rule = compile({ forbid: { ast: { language: "python", rule: { kind: "string", parse: {
    grammar: "pythonFString", interpolation: "{...}", kind: "template", value: { regex: "SELECT \\{\\.\\.\\.\\} FROM" },
  } } } } });
  const result = evaluate(rule, "x = f'SELECT {name} FROM table'\ny = f\"{get('SELECT FROM')}\"\nz = f'SELECT {x:{width}} FROM t'\n");
  expect(result.failures).toEqual([]);
  expect(result.violations.map((v: { span: { startLine: number } }) => v.span.startLine)).toEqual([0, 2]);
});

it("enumerates decoded f-string parts and keeps their individual host lines", () => {
  const rule = compile({ forbid: { ast: { language: "python", rule: { kind: "string", parse: {
    grammar: "pythonFString", has: { kind: "literal", value: { regex: "(?s)^.{2}" }, capture: "PART" },
  } } } }, violation: { code: "CHECK", anchor: "capture.PART", each: "PART", message: "part" } });
  const result = evaluate(rule, "x = f'''first {x}\nsecond {y} third'''\n");
  expect(result.failures).toEqual([]);
  expect(result.violations).toHaveLength(3);
  expect(result.violations.map((v: { span: { startLine: number } }) => v.span.startLine)).toEqual([0, 0, 1]);
});

it("keeps different interpolation projections separate in the parse cache", () => {
  const clauses = ["?", "{...}"].map((marker, index) => ({ id: `projection-${index}`, forbid: { ast: {
    language: "python", rule: { kind: "string", parse: { grammar: "pythonFString", interpolation: marker,
      value: { eq: `before${marker}after` } } },
  } }, violation: { code: "CHECK", anchor: "witness.span", message: "projection" } }));
  const rule = native.CodeRuleDslRuntime.compile(JSON.stringify({ format: "rerail.policy/v1", id: "projection-cache",
    revision: 1, mode: "steer", clauses }), "json", capabilities);
  const result = evaluate(rule, "x = f'before{x}after'\n");
  expect(result.failures).toEqual([]);
  expect(result.violations).toHaveLength(2);
});


it("still enforces the explicit closure operation limit", () => {
  const evidence = { ...closureClause.budget.query.evidence, closure: [{ name: "known",
    seeds: Array.from({ length: 100 }, (_, index) => `Seed${index}`), contains: { known: "member" } }] };
  const result = evaluate(compile({ budget: { ...closureClause.budget, query: { evidence } } }, { maxVisitedNodes: 32 }), "A = Root\n");
  expect(result.violations).toEqual([]);
  expect(result.failures.some((failure: { code: string; message: string }) => failure.code === "budget-exceeded" && failure.message.includes("name closure"))).toBe(true);
});

it("rejects invalid row anchors and malformed capture projection paths", () => {
  const evidence = closureClause.budget.query.evidence;
  expect(() => compile({ budget: { ...closureClause.budget, query: { evidence: { ...evidence, anchor: false } } } })).toThrow();
  for (const source of ["capture.FROM.text.text", "capture.FROM.unknown.startByte", "capture..startLine", "capture.FROM."]) {
    expect(() => compile({ budget: { ...closureClause.budget, query: { evidence: {
      ...evidence, project: { ...evidence.project, from: source },
    } } } })).toThrow("invalid capture projection");
  }
});
