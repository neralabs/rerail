import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { codeRuleSchema, loadNativeBinding } from "../../../src/engine/index.js";

const native = loadNativeBinding();
const capabilities = readFileSync(new URL("../../../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8");

it("authors constructor field conventions and preserves unknown annotation shadowing", async () => {
  const policy = (id: string, constructorFields?: { fieldToParameter: { replace: string; by: string } }) => ({
    format: "rerail.policy/v1", id, revision: 1, mode: "log", clauses: [{ id: "reference", forbid: { ast: { language: "python", rule: {
      kind: "function_definition", all: [{ has: { field: "name", pattern: "run" } }, { calls: {
        graph: "references", at: "definition", constructorFields, kind: "function_definition", has: { field: "name", pattern: "_query" },
      } }],
    } } }, violation: { code: "REFERENCE", message: "References a query", anchor: "witness.span" } }],
  });
  const convention = { fieldToParameter: { replace: "^state_", by: "" } };
  const compile = (id: string, fields?: typeof convention) => {
    const document = policy(id, fields); expect(codeRuleSchema.safeParse(document).success).toBe(true);
    return native.CodeRuleDslRuntime.compile(JSON.stringify(document), "json", capabilities);
  };
  const linked = compile("constructor-contract", convention), plain = compile("plain-fields");
  const source = "class DB:\n def _query(self, x): return x\n def read(self, x): return self._query(x)\nclass Pure:\n def read(self, x): return x\nclass Client:\n def __init__(self, dependency: DB): pass\n def run(self): return self.state_dependency.read(1)\n";
  const files = [
    { path: "impl/client.py", language: "python", source },
    { path: "api/use.py", language: "python", source: "from impl.client import Client\ndef run(client: Client): return client.run()\n" },
  ];
  const snapshot = JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files, facts: [] });
  const expected = JSON.parse(linked.evaluate(snapshot, "report"));
  expect(expected.failures).toEqual([]); expect(expected.violations).toHaveLength(2);
  expect(JSON.parse(plain.evaluate(snapshot, "report"))).toMatchObject({ violations: [], failures: [] });
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(linked); workspace.addCodeRule(plain);
    const evaluate = async (revision: number, source: string) => JSON.parse(await workspace.applyAsync(JSON.stringify({ revision,
      files: revision === 1 ? files : [{ ...files[0]!, source }], removed: [] }))).evaluation;
    expect(await evaluate(1, source)).toMatchObject({ violations: expected.violations, failures: [] });
    expect(await evaluate(2, source.replace("dependency: DB", "dependency: Pure"))).toMatchObject({ violations: [], failures: [] });
    const unknown = source.replace("class Client:\n", "class Client:\n state_dependency: DB\n").replace("dependency: DB)", "dependency: Missing)");
    const result = await evaluate(3, unknown);
    expect(result.failures).toEqual([]); expect(result.violations).toHaveLength(2);
    expect(result.violations.every((v: { ruleId: string }) => v.ruleId === "plain-fields")).toBe(true);
  } finally { workspace.close(); }
});

it("invalidates a declared bare-name fallback when an imported helper is renamed", async () => {
  const policy = { format: "rerail.policy/v1", id: "declared-fallback", revision: 1, mode: "log", clauses: [{
    id: "reference", forbid: { ast: { language: "python", rule: { kind: "function_definition", all: [
      { has: { field: "name", pattern: "run" } },
      { calls: { graph: "references", at: "definition", kind: "function_definition", has: { field: "name", pattern: "_query" } } },
    ] } } }, violation: { code: "REFERENCE", message: "References a query", anchor: "witness.span" },
  }] };
  const rule = native.CodeRuleDslRuntime.compile(JSON.stringify(policy), "json", capabilities);
  const files = [
    { path: "impl/query.py", language: "python", source: "def _query(): return 1\ndef helper(): return _query()\n" },
    { path: "impl/service.py", language: "python", source: "from impl.query import helper\nclass Service:\n def run(self): return helper()\n" },
    { path: "api/use.py", language: "python", source: "from impl.service import Service\ndef run(service: Service): return service.run()\n" },
  ];
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(rule);
    const evaluate = async (revision: number, changed: typeof files) => JSON.parse(await workspace.applyAsync(JSON.stringify({ revision, files: changed, removed: [] }))).evaluation;
    const first = await evaluate(1, files);
    expect(first.failures).toEqual([]);
    expect(first.violations).toHaveLength(2);
    const changed = { ...files[0]!, source: files[0]!.source.replace("def helper()", "def other()") };
    expect(await evaluate(2, [changed])).toMatchObject({ violations: [], failures: [] });
  } finally { workspace.close(); }
});

it("authors decorated sibling delegation without leaking it into another graph", async () => {
  const links = [{ decorator: "^compat$", name: { replace: "(?:_async)?$", by: "_deferred" } }];
  const policy = (id: string, siblingLinks?: typeof links) => ({ format: "rerail.policy/v1", id, revision: 1, mode: "log", clauses: [{
    id: "reference", forbid: { ast: { language: "python", rule: { kind: "function_definition", all: [
      { has: { field: "name", regex: "^(read|read_async|run)$" } },
      { calls: { graph: "references", at: "definition", siblingLinks, kind: "function_definition", has: { field: "name", pattern: "_query" } } },
    ] } } }, violation: { code: "REFERENCE", message: "References a query", anchor: "witness.span" },
  }] });
  const compile = (id: string, siblingLinks?: typeof links) => {
    const document = policy(id, siblingLinks);
    expect(codeRuleSchema.safeParse(document).success).toBe(true);
    return native.CodeRuleDslRuntime.compile(JSON.stringify(document), "json", capabilities);
  };
  const linked = compile("with-contract", links), plain = compile("without-contract");
  const source = "class Service:\n def _query(self): return 1\n def read_deferred(self): return self._query()\n @compat\n def read(self): return None\n @compat\n def read_async(self): return None\n def run(self): return self.read()\n";
  const files = [
    { path: "impl/service.py", language: "python", source },
    { path: "api/use.py", language: "python", source: "from impl.service import Service\ndef run(client: Service): return client.run()\n" },
  ];
  const snapshot = JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files, facts: [] });
  const expected = JSON.parse(linked.evaluate(snapshot, "report"));
  expect(expected.failures).toEqual([]);
  expect(expected.violations).toHaveLength(4);
  expect(JSON.parse(plain.evaluate(snapshot, "report"))).toMatchObject({ violations: [], failures: [] });
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(linked); workspace.addCodeRule(plain);
    const evaluate = async (revision: number, changed: typeof files) => JSON.parse(await workspace.applyAsync(JSON.stringify({ revision, files: changed, removed: [] }))).evaluation;
    expect(await evaluate(1, files)).toMatchObject({ violations: expected.violations, failures: [] });
    const result = await evaluate(2, [{ ...files[0]!, source: source.replace("@compat\n def read(self)", "@other\n def read(self)") }]);
    expect(result.failures).toEqual([]);
    expect(result.violations).toEqual([expect.objectContaining({ ruleId: "with-contract", path: "impl/service.py", span: expect.objectContaining({ startLine: 6 }) })]);
  } finally { workspace.close(); }
});

it("validates sibling contracts and applies regex limits even after a contract was interned", () => {
  const policy = (siblingLinks: unknown) => JSON.stringify({ format: "rerail.policy/v1", id: "contract-validation", revision: 1, mode: "log", clauses: [{
    id: "reference", forbid: { ast: { language: "python", rule: { kind: "function_definition", calls: {
      graph: "references", at: "definition", siblingLinks, kind: "function_definition", has: { field: "name", pattern: "_query" },
    } } } }, violation: { code: "REFERENCE", message: "Reference", anchor: "witness.span" },
  }] });
  for (const links of [[], [{ decorator: "[", name: { replace: "$", by: "_deferred" } }],
    [{ decorator: "compat", name: { replace: "[", by: "_deferred" } }],
    [{ decorator: "compat", name: { replace: "$", by: 1 } }],
    [{ decorator: "compat", name: { replace: "$", by: "_deferred" }, unknown: true }]]) {
    expect(() => native.CodeRuleDslRuntime.compile(policy(links), "json", capabilities)).toThrow();
  }
  const document = policy([{ decorator: "^compat$", name: { replace: "$", by: "_deferred" } }]);
  native.CodeRuleDslRuntime.compile(document, "json", capabilities);
  const limited = JSON.parse(capabilities); limited.limits.maxRegexPatternBytes = 4;
  expect(() => native.CodeRuleDslRuntime.compile(document, "json", JSON.stringify(limited))).toThrow(/siblingLinks\/0\/decorator/);
});

it("follows declared field types and super methods and invalidates unchanged consumers", async () => {
  const policy = { format: "rerail.policy/v1", id: "declared-fields", revision: 1, mode: "log", clauses: [{
    id: "reference", forbid: { ast: { language: "python", rule: { kind: "function_definition", all: [
      { has: { field: "name", regex: "^(read|run)$" } },
      { calls: { graph: "references", at: "definition", kind: "function_definition", has: { field: "name", regex: "^_query$" } } },
    ] } } }, violation: { code: "REFERENCE", message: "References a query", anchor: "witness.span" },
  }] };
  const rule = native.CodeRuleDslRuntime.compile(JSON.stringify(policy), "json", capabilities);
  const source = "class DB:\n def _query(self, x): return x\n def read(self, x): return self._query(x)\nclass Pure:\n def read(self, x): return x\nclass Holder:\n db: DB\n def read(self, x): return self.db.read(x)\nclass Child(Holder):\n def run(self, x): return super().read(x)\n";
  const files = [
    { path: "impl/query.py", language: "python", source },
    { path: "api/use.py", language: "python", source: "from impl.query import Child\ndef run(client: Child): return client.run(1)\n" },
  ];
  const snapshot = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files, facts: [] }), "report"));
  expect(snapshot.failures).toEqual([]);
  expect(snapshot.violations).toHaveLength(4);
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(rule);
    const evaluate = async (revision: number, changed: typeof files) => JSON.parse(await workspace.applyAsync(JSON.stringify({ revision, files: changed, removed: [] }))).evaluation;
    expect(await evaluate(1, files)).toMatchObject({ violations: snapshot.violations, failures: [] });
    const result = await evaluate(2, [{ ...files[0]!, source: source.replace("db: DB", "db: Pure") }]);
    expect(result.failures).toEqual([]);
    expect(result.violations).toEqual([expect.objectContaining({ path: "impl/query.py", span: expect.objectContaining({ startLine: 2 }) })]);
  } finally { workspace.close(); }
});

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
