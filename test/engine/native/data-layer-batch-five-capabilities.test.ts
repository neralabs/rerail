import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { codeRuleSchema, loadNativeBinding } from "../../../src/engine/index.js";

const native = loadNativeBinding();
const capabilities = readFileSync(new URL("../../../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8");

it("validates the declared-reference query interface in both compilers", () => {
  for (const calls of [
    { graph: "missing", at: "definition", kind: "function_definition" },
    { graph: "references", at: "missing", kind: "function_definition" },
    { graph: "references", at: "call", kind: "function_definition" },
    { graph: "references", kind: "function_definition" },
    { graph: "references", at: "definition", kind: "function_definition", unknown: true },
  ]) {
    const policy = { format: "rerail.policy/v1", id: "invalid-reference", revision: 1, mode: "log", clauses: [{
      id: "reference", forbid: { ast: { language: "python", rule: { kind: "function_definition", calls } } },
      violation: { code: "REFERENCE", message: "Reference", anchor: "witness.span" },
    }] };
    expect(codeRuleSchema.safeParse(policy).success).toBe(false);
    expect(() => native.CodeRuleDslRuntime.compile(JSON.stringify(policy), "json", capabilities)).toThrow();
  }
});

it("selects final declarations in a large class within the existing work limit", async () => {
  const policy = readFileSync(new URL("../../../packs/data-layer/candidates/rules/data-layer-imperative-private-functions.json", import.meta.url), "utf8");
  const rule = native.CodeRuleDslRuntime.compile(policy, "json", capabilities);
  const files = [{ path: "impl/query.py", language: "python", source: "class Query:\n" + Array.from({ length: 700 },
    (_, index) => ` def _helper_${index}(self, x): return x\n`).join("") +
    " def _overloaded(self, x): return execute(x)\n @pure_lazy\n def _overloaded(self, x): return x\n" }];
  const snapshot = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files, facts: [] }), "report"));
  expect(snapshot.failures).toEqual([]);
  expect(snapshot.violations).toHaveLength(700);
  expect(snapshot.violations.every((v: { code: string }) => v.code === "eager_signature")).toBe(true);
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(rule);
    const result = JSON.parse(await workspace.applyAsync(JSON.stringify({ revision: 1, files, removed: [] }))).evaluation;
    expect(result).toMatchObject({ failures: [] });
    // The workspace's first result page may be smaller than the full result.
    let count = 0;
    for (let cursor = 0; ;) {
      const page = JSON.parse(await workspace.resultPageAsync(cursor, 1024 * 1024));
      count += page.violations.length;
      expect(page.failures).toEqual([]);
      if (page.done) break;
      expect(page.cursor).toBeGreaterThan(cursor);
      cursor = page.cursor;
    }
    expect(count).toBe(700);
  } finally { workspace.close(); }
});

it("follows declared symbol references through callbacks and returned values", async () => {
  const policy = { format: "rerail.policy/v1", id: "reference-graph", revision: 1, mode: "log", clauses: [{
    id: "reference", forbid: { ast: { language: "python", rule: { kind: "function_definition", all: [
      { has: { field: "name", regex: "^(run|returned)$" } },
      { calls: { graph: "references", at: "definition", through: "any", kind: "function_definition", has: { field: "name", regex: "^_query$" } } },
    ] } } }, violation: { code: "REFERENCE", message: "References a query helper", anchor: "witness.span" },
  }] };
  const rule = native.CodeRuleDslRuntime.compile(JSON.stringify(policy), "json", capabilities);
  const callableClass = "class CallableQuery:\n def __call__(self): return _query(0)\n def run(self): self.app = 1\n";
  const files = [
    { path: "impl/query.py", language: "python", source: "def _query(x): return x\ndef run(xs): return map(_query, xs)\ndef returned(): return _query\n" + callableClass },
    { path: "impl/cache.py", language: "python", source: "class Cache:\n def invalidate(self, tag): ...\n def run(self, tag): return self.invalidate(tag)\nclass QueryCache(Cache):\n def _query(self, tag): return tag\n def invalidate(self, tag): return self._query(tag)\n" },
    { path: "impl/consumer.py", language: "python", source: "from impl.query import returned\ndef run(): return returned()\n" },
  ];
  const snapshot = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files, facts: [] }), "report"));
  expect(snapshot.failures).toEqual([]);
  expect(snapshot.violations.map((v: { path: string; span: { startLine: number } }) => [v.path, v.span.startLine]).sort()).toEqual([
    ["impl/consumer.py", 1], ["impl/query.py", 1], ["impl/query.py", 2],
  ]);
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(rule);
    const evaluate = async (revision: number, changed: typeof files) => JSON.parse(await workspace.applyAsync(JSON.stringify({ revision, files: changed, removed: [] }))).evaluation;
    expect(await evaluate(1, files)).toMatchObject({ violations: snapshot.violations, failures: [] });
    const partial = await evaluate(2, [{ ...files[0]!, source: "def _query(x): return x\ndef run(xs): return map(_query, xs)\ndef returned(): return None\n" + callableClass }]);
    expect(partial.failures).toEqual([]);
    expect(partial.violations).toEqual([expect.objectContaining({ path: "impl/query.py", span: expect.objectContaining({ startLine: 1 }) })]);
    expect(await evaluate(3, [{ ...files[0]!, source: "def _query(x): return x\ndef run(xs): return xs\ndef returned(): return None\n" + callableClass }])).toMatchObject({ violations: [], failures: [] });
  } finally { workspace.close(); }
});

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
