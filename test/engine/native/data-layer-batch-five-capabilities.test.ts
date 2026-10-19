import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { loadNativeBinding } from "../../../src/engine/index.js";

const native = loadNativeBinding();
const capabilities = readFileSync(new URL("../../../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8");

it("scopes call-graph seeds by their own file path", () => {
  const policy = { format: "rerail.policy/v1", id: "path-seed", revision: 1, mode: "log", clauses: [{
    id: "caller", forbid: { ast: { language: "python", rule: { kind: "call", calls: {
      through: "any", kind: "function_definition", filePath: { regex: "(?:^|/)query\\.py$" },
      has: { field: "name", regex: "^_" },
    } } } }, violation: { code: "CALLER", message: "Calls a query helper", anchor: "witness.span" },
  }] };
  const rule = native.CodeRuleDslRuntime.compile(JSON.stringify(policy), "json", capabilities);
  const result = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, facts: [], files: [
    { path: "impl/query.py", language: "python", source: "def _helper(x): return x\n" },
    { path: "impl/pure.py", language: "python", source: "def _helper(x): return x\n" },
    { path: "api/use.py", language: "python", source: "from impl.query import _helper\nfrom impl.pure import _helper as pure\ndef run(x):\n return _helper(x) + pure(x)\n" },
  ] }), "report"));
  expect(result.failures).toEqual([]);
  expect(result.violations).toEqual([expect.objectContaining({ code: "CALLER", path: "api/use.py", span: expect.objectContaining({ startLine: 3 }) })]);
});

it("compares relation counts without exporting captures from the counted rule", () => {
  const policy = { format: "rerail.policy/v1", id: "parameter-count", revision: 1, mode: "log", clauses: [{
    id: "different", forbid: { ast: { language: "python", rule: { kind: "module", all: [
      { has: { kind: "function_definition", has: { field: "parameters", count: { has: { kind: "identifier" }, max: 100, as: "LEFT" } } } },
      { has: { kind: "class_definition", has: { stopBy: "end", kind: "parameters", count: { has: { kind: "identifier" }, max: 100, as: "RIGHT" } } } },
      { compare: { left: "LEFT", right: "RIGHT", op: "ne" } },
    ] } } }, violation: { code: "COUNT", message: "Different counts", anchor: "witness.span" },
  }] };
  const rule = native.CodeRuleDslRuntime.compile(JSON.stringify(policy), "json", capabilities);
  const evaluate = (source: string) => JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0,
    files: [{ path: "a.py", language: "python", source }], facts: [] }), "report"));
  expect(evaluate("def f(x): pass\nclass C:\n def g(x, y): pass\n")).toMatchObject({ violations: [expect.objectContaining({ code: "COUNT" })], failures: [] });
  expect(evaluate("def f(x): pass\nclass C:\n def g(x): pass\n")).toMatchObject({ violations: [], failures: [] });
});

it("compares layer numbers extracted from file paths and import captures", async () => {
  const policy = { format: "rerail.policy/v1", id: "layer-comparison", revision: 1, mode: "log", clauses: [{
    id: "upward", budget: { query: { evidence: { as: "row", query: { ast: { language: "python", rule: {
      kind: "import_from_statement", has: { field: "module_name", pattern: "$MODULE" },
    } } }, project: { path: "file.path",
      from: { source: "file.path", regex: "(?:^|/)l([0-9]{4})_", group: 1 },
      to: { source: "capture.MODULE", regex: "\\.l([0-9]{4})_", group: 1 },
    }, where: { lt: ["row.from", "row.to"] } } }, count: { lte: 0 }, group_by: ["row.path"] },
    violation: { code: "UPWARD", anchor: "witness.span", message: "upward dependency" },
  }] };
  const rule = native.CodeRuleDslRuntime.compile(JSON.stringify(policy), "json", capabilities);
  const files = [
    { path: "api/l2000_low/a.py", language: "python", source: "from api.l3000_high import x\n" },
    { path: "impl/l5000_high/a.py", language: "python", source: "from api.l3000_high import x\n" },
    { path: "api/no_layer.py", language: "python", source: "from api.l3000_high import x\n" },
  ];
  const snapshot = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files, facts: [] }), "report"));
  expect(snapshot.failures).toEqual([]);
  expect(snapshot.violations.map((v: { path: string }) => v.path)).toEqual([files[0]!.path]);
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(rule);
    const apply = async (revision: number, changed: typeof files) => JSON.parse(await workspace.applyAsync(JSON.stringify({ revision, files: changed, removed: [] }))).evaluation;
    expect(await apply(1, files)).toMatchObject({ violations: snapshot.violations, failures: [] });
    expect(await apply(2, [{ ...files[0]!, source: "from api.l1000_base import x\n" }])).toMatchObject({ violations: [], failures: [] });
  } finally { workspace.close(); }
});

it("rejects invalid or unbounded projection extraction specifications", () => {
  const policy = (projection: unknown) => JSON.stringify({ format: "rerail.policy/v1", id: "invalid-projection", revision: 1,
    mode: "log", clauses: [{ id: "projection", budget: { query: { evidence: { as: "row", query: { ast: {
      language: "python", rule: { kind: "module" },
    } }, project: { value: projection } } }, count: { lte: 0 } },
    violation: { code: "BAD", message: "Bad projection", anchor: "witness.span" } }] });
  for (const projection of [
    { source: "file.path", regex: "(x)", group: 2 },
    { source: "file.path", regex: "(x)", group: -1 },
    { source: "file.path", regex: "[", group: 0 },
    { source: "file.path", regex: "x".repeat(16385), group: 0 },
    { source: "file.path", regex: "x", group: 0, extra: true },
  ]) expect(() => native.CodeRuleDslRuntime.compile(policy(projection), "json", capabilities)).toThrow();
});
