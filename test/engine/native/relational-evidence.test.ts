import { readFileSync } from "node:fs";
import type { CodeRuleDslRelation } from "../../../src/engine/code-rule/code-rule.js";
import { expect, it } from "vitest";
import { codeRuleSchema, loadNativeBinding } from "../../../src/engine/index.js";

const native = loadNativeBinding();
const capabilities = readFileSync(new URL("../../../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8");
const input: CodeRuleDslRelation = { evidence: { as: "f", query: { ast: { language: "python", rule: {
  kind: "function_definition", has: { field: "name", pattern: "$NAME" },
} } }, project: { name: "capture.NAME" } } };
const policy = (query: string, key = ["family"]) => ({
  format: "rerail.policy/v1", id: "relational-evidence", revision: 1, mode: "log",
  clauses: [{ id: "coverage", relational: { inputs: { functions: input }, query, key, code: "issue" },
    violation: { code: "COVERAGE", message: "Missing companion", anchor: "witness.span" } }],
});
const query = `SELECT f.__path AS path, f.__start AS start_byte, f.__end AS end_byte,
  f.name AS family, 'missing_companion' AS issue
  FROM functions f LEFT JOIN functions g ON g.name = f.name || '_async'
  WHERE f.name NOT LIKE '%_async' AND g.name IS NULL`;
const compile = (document: unknown) => native.CodeRuleDslRuntime.compile(JSON.stringify(document), "json", capabilities);

it("counts shared projected evidence once and releases its census on edits and removal", async () => {
  const base = policy("SELECT __path AS path,__start AS start_byte,__end AS end_byte,name AS family,'unused' AS issue FROM functions WHERE 0");
  const document = { ...base, clauses: [{ ...base.clauses[0], relational: { ...base.clauses[0]!.relational, partitionBy: "repository" } }] };
  const workspace = new native.CodeRuleDslWorkspace();
  const memory = () => JSON.parse(workspace.memoryProfile()).ownership;
  const apply = async (revision: number, name?: string) => {
    const result = JSON.parse(await workspace.applyAsync(JSON.stringify({ revision,
      files: name === undefined ? [] : [{ path: "a.py", language: "python", source: `def ${name}(): pass\n` }],
      removed: name === undefined ? ["a.py"] : [], quiet: true })));
    expect(result.evaluation.failures).toEqual([]);
  };
  try {
    workspace.addCodeRule(compile(document));
    await apply(1, "x".repeat(2048));
    expect(memory()).toMatchObject({ witnesses: 2, projectionReferences: 2, uniqueProjections: 1,
      projection: { objectEntries: 1, stringBytes: 2048 } });
    await apply(2, "short");
    expect(memory()).toMatchObject({ projectionReferences: 2, uniqueProjections: 1, projection: { stringBytes: 5 } });
    await apply(3);
    expect(memory()).toMatchObject({ witnesses: 0, uniqueProjections: 0, projection: { stringBytes: 0 } });
    workspace.close();
    expect(memory().uniqueProjections).toBe(0);
  } finally { workspace.close(); }
});

it("preserves nullable and numeric projections across SQLite insertion batches and tails", () => {
  const document = policy("SELECT __path AS path,__start AS start_byte,__end AS end_byte,name AS family,'batch' AS issue,annotation,offset FROM functions");
  const evidence = { as: "f", query: { ast: { language: "python", rule: { all: [
    { kind: "function_definition" }, { has: { field: "name", pattern: "$NAME" } },
    { any: [{ has: { field: "return_type", pattern: "$TYPE" } }, { pattern: "$_" }] },
  ] } } }, project: { name: "capture.NAME", annotation: "capture.TYPE", offset: "capture.NAME.startByte" } };
  const rule = compile({ ...document, clauses: [{ ...document.clauses[0], relational: {
    ...document.clauses[0]!.relational, inputs: { functions: { evidence } },
  } }] });
  const sources = [Array.from({ length: 131 }, (_, i) => `def read_ä${i}()${i % 2 ? " -> str" : ""}: pass`).join("\n"), "value = 'clean'\n"];
  for (const source of sources) {
    const snapshot = JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0,
      files: [{ path: "unicode/ü.py", language: "python", source }] });
    const actual = JSON.parse(rule.evaluate(snapshot, "report"));
    expect(actual.failures).toEqual([]);
    expect(actual.violations).toHaveLength(source.startsWith("def") ? 131 : 0);
    for (const violation of actual.violations) {
      const row = violation.variables.row;
      const index = Number(row.family.slice("read_ä".length));
      expect(row.annotation).toBe(index % 2 ? "str" : null);
      expect(row.offset).toBe(Buffer.byteLength(source.slice(0, source.indexOf(row.family))));
    }
  }
});

it("accepts 64 shared SQL stages and rejects 65 in both schema and native compilation", () => {
  const document = policy("SELECT __path AS path,__start AS start_byte,__end AS end_byte,name AS family,'shared' AS issue FROM level_63");
  const stages = Array.from({ length: 64 }, (_, i) => ({ name: `level_${i}`, query: `SELECT * FROM ${i ? `level_${i - 1}` : "functions"}` }));
  const withStages = { ...document, clauses: [{ ...document.clauses[0], relational: { ...document.clauses[0]!.relational, stages } }] };
  expect(codeRuleSchema.safeParse(withStages).success).toBe(true);
  const rule = compile(withStages);
  const result = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0,
    files: [{ path: "a.py", language: "python", source: "def read(): pass\n" }] }), "report"));
  expect(result.failures).toEqual([]);
  expect(result.violations).toHaveLength(1);
  stages.push({ name: "overflow", query: "SELECT * FROM level_63" });
  expect(codeRuleSchema.safeParse(withStages).success).toBe(false);
  expect(() => compile(withStages)).toThrow("at most 64 stages");
});

it.each([{ locations: ["missing.py"] }, { references: { language: "python" } }])("rejects project-only inputs on file partitions: %j", extra => {
  const base = policy(query);
  const document = { ...base, clauses: [{ ...base.clauses[0], relational: { ...base.clauses[0]!.relational, partitionBy: "file", ...extra } }] };
  expect(codeRuleSchema.safeParse(document).success).toBe(false);
  expect(() => compile(document)).toThrow("file partitions cannot declare references or locations");
});

it("partitions a declared local relation by file on snapshot, edit, and removal", async () => {
  const base = policy(query);
  const document = { ...base, clauses: [{ ...base.clauses[0], relational: { ...base.clauses[0]!.relational, partitionBy: "file" } }] };
  expect(codeRuleSchema.safeParse(document).success).toBe(true);
  const rule = compile(document);
  const files = [
    { path: "a.py", language: "python", source: "def read(): pass\n" },
    { path: "b.py", language: "python", source: "def read_async(): pass\n" },
  ];
  const evaluate = (current: typeof files) => JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files: current }), "report"));
  const initial = evaluate(files);
  expect(initial.failures).toEqual([]); expect(initial.violations).toHaveLength(1);
  expect(initial.violations[0].path).toBe("a.py");
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(rule);
    const apply = async (revision: number, changed: typeof files, removed: string[] = []) => JSON.parse(await workspace.applyAsync(JSON.stringify({ revision, files: changed, removed }))).evaluation;
    expect(await apply(1, files)).toMatchObject({ failures: [], violations: initial.violations });
    files[1] = { ...files[1]!, source: "def other(): pass\n" };
    expect(await apply(2, [files[1]!])).toMatchObject({ failures: [], violations: evaluate(files).violations });
    expect(await apply(3, [], ["a.py"])).toMatchObject({ failures: [], violations: evaluate([files[1]!]).violations });
  } finally { workspace.close(); }
});

it("validates a deep stage graph without expanding earlier stages as nested views", () => {
  const stages = Array.from({ length: 20 }, (_, index) => {
    const previous = index === 0 ? "functions" : `level_${index - 1}`;
    return { name: `level_${index}`, query: `SELECT a.* FROM ${previous} a
      JOIN ${previous} b ON b.__path=a.__path AND b.__start=a.__start AND b.__end=a.__end` };
  });
  const document = policy("SELECT __path AS path,__start AS start_byte,__end AS end_byte,name AS family,'deep' AS issue FROM level_19");
  const withStages = { ...document, clauses: [{ ...document.clauses[0], relational: { ...document.clauses[0]!.relational, stages } }] };
  const rule = compile(withStages);
  const result = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0,
    files: [{ path: "a.py", language: "python", source: "def read(): pass\n" }] }), "report"));
  expect(result.failures).toEqual([]); expect(result.violations).toHaveLength(1);
});

it("joins projected source rows and invalidates an unchanged file when its companion changes", async () => {
  const document = policy(query);
  expect(codeRuleSchema.safeParse(document).success).toBe(true);
  const rule = compile(document);
  const files = [
    { path: "api/first.py", language: "python", source: "def read(): return 1\n" },
    { path: "api/second.py", language: "python", source: "def other(): return 2\n" },
  ];
  const snapshot = JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files });
  const expected = JSON.parse(rule.evaluate(snapshot, "report"));
  expect(expected.failures).toEqual([]); expect(expected.violations).toHaveLength(2);
  expect(expected.violations.every((v: { code: string }) => v.code === "missing_companion")).toBe(true);
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(rule);
    const evaluate = async (revision: number, changed: typeof files) => JSON.parse(await workspace.applyAsync(JSON.stringify({ revision, files: changed, removed: [] }))).evaluation;
    expect(await evaluate(1, files)).toMatchObject({ failures: [], violations: expected.violations });
    const after = await evaluate(2, [{ ...files[1]!, source: "def read_async(): return 2\n" }]);
    expect(after).toMatchObject({ failures: [], violations: [] });
  } finally { workspace.close(); }
});

it("keeps distinct family identities that report the same source span", () => {
  const rule = compile(policy(`SELECT __path AS path, __start AS start_byte, __end AS end_byte,
    'first' AS family, 'missing' AS issue FROM functions UNION ALL
    SELECT __path, __start, __end, 'second', 'missing' FROM functions`));
  const result = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0,
    files: [{ path: "a.py", language: "python", source: "def read(): pass\n" }] }), "report"));
  expect(result.failures).toEqual([]); expect(result.violations).toHaveLength(2);
  expect(new Set(result.violations.map((v: { id: string }) => v.id)).size).toBe(2);
});

it.each(["DELETE FROM functions", "SELECT 1; SELECT 2", "SELECT random()", "SELECT * FROM pragma_database_list"])(
  "refuses queries outside the deterministic read-only relation contract: %s", sql => {
    expect(() => compile(policy(sql))).toThrow();
  },
);

it("projects decoded literal transforms into relational fields", () => {
  const document = policy("SELECT __path AS path,__start AS start_byte,__end AS end_byte,name AS family,'literal' AS issue FROM functions WHERE name='read'");
  document.clauses[0]!.relational.inputs.functions = { evidence: { as: "f", query: { ast: { language: "python", rule: {
    kind: "string", pattern: "$VALUE",
  }, transform: { TEXT: { replace: { source: "$VALUE", value: true, replace: "\\b\\B", by: "" } } } } },
  project: { name: "capture.TEXT.text" } } };
  const rule = compile(document);
  const result = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0,
    files: [{ path: "a.py", language: "python", source: "names=[u'read', '\\x72ead']\n" }] }), "report"));
  expect(result.failures).toEqual([]); expect(result.violations).toHaveLength(2);
});

it("keeps suppressed rows as evidence and suppresses only their reported anchors", async () => {
  const rule = compile(policy(query));
  const files = [{ path: "a.py", language: "python", source: "def read(): pass\n# rerail-ignore relational-evidence\ndef read_async(): pass\n# rerail-ignore relational-evidence\ndef other(): pass\n" }];
  const snapshot = JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files });
  expect(JSON.parse(rule.evaluate(snapshot, "report"))).toMatchObject({ failures: [], violations: [] });
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(rule);
    expect(JSON.parse(await workspace.applyAsync(JSON.stringify({ revision: 1, files, removed: [] }))).evaluation).toMatchObject({ failures: [], violations: [] });
  } finally { workspace.close(); }
});

it("bounds recursive queries and reports exhaustion as a failure", () => {
  const rule = compile(policy(`WITH RECURSIVE forever(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM forever)
    SELECT f.__path AS path,f.__start AS start_byte,f.__end AS end_byte,'family' AS family,'loop' AS issue
    FROM functions f,forever WHERE x<0`));
  const result = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0,
    files: [{ path: "a.py", language: "python", source: "def read(): pass\n" }] }), "report"));
  expect(result.violations).toEqual([]); expect(result.failures).toEqual([expect.objectContaining({ code: "relational-failed", message: expect.stringContaining("interrupted") })]);
});

it("joins native reference edges and invalidates them after an edit in another file", async () => {
  const document = policy(`SELECT __path AS path,__start AS start_byte,__end AS end_byte,
    source_name || target_name AS family,'reference' AS issue FROM reference_edges WHERE target_name LIKE '%._effect'`);
  Object.assign(document.clauses[0]!.relational, { references: { language: "python" } });
  const rule = compile(document);
  const files = [
    { path: "pkg/effect.py", language: "python", source: "def _effect(x): return x\n" },
    { path: "pkg/use.py", language: "python", source: "from pkg.effect import _effect\ndef run(xs): return map(_effect,xs)\n" },
  ];
  const snapshot = JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files });
  const expected = JSON.parse(rule.evaluate(snapshot,"report"));
  expect(expected.failures).toEqual([]); expect(expected.violations).toHaveLength(1);
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(rule);
    const evaluate = async (revision: number, changed: typeof files) => JSON.parse(await workspace.applyAsync(JSON.stringify({ revision, files: changed, removed: [] }))).evaluation;
    expect(await evaluate(1,files)).toMatchObject({ failures: [], violations: expected.violations });
    expect(await evaluate(2,[{ ...files[1]!,source:files[1]!.source.replace("_effect,xs","unknown,xs") }])).toMatchObject({ failures: [], violations: [] });
    expect(await evaluate(3,[files[1]!])).toMatchObject({ failures: [], violations: expected.violations });
    expect(await evaluate(4,[{ ...files[0]!,source:"def another(x): return x\n" }])).toMatchObject({ failures: [], violations: [] });
  } finally { workspace.close(); }
});

it("batches reference edges across full batches and tails and repairs edits", async () => {
  const document = policy(`SELECT __path AS path,__start AS start_byte,__end AS end_byte,
    source_name || target_name AS family,'reference' AS issue FROM reference_edges WHERE target_name LIKE '%._effect'`);
  Object.assign(document.clauses[0]!.relational, { references: { language: "python" } });
  const rule = compile(document);
  const files = [
    { path: "pkg/effect.py", language: "python", source: "def _effect(x): return x\n" },
    { path: "pkg/use.py", language: "python", source: "from pkg.effect import _effect\n" +
      Array.from({ length: 131 }, (_, i) => `def run${i}(xs): return map(_effect,xs)`).join("\n") },
  ];
  const evaluate = () => JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files }), "report"));
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    const expected = evaluate();
    expect(expected.failures).toEqual([]);
    expect(expected.violations).toHaveLength(131);
    workspace.addCodeRule(rule);
    const apply = async (revision: number, changed: typeof files, removed: string[] = []) =>
      JSON.parse(await workspace.applyAsync(JSON.stringify({ revision, files: changed, removed }))).evaluation;
    expect(await apply(1, files)).toMatchObject({ failures: [], violations: expected.violations });
    expect(await apply(2, [{ ...files[1]!, source: "pass\n" }])).toMatchObject({ failures: [], violations: [] });
    expect(await apply(3, [files[1]!])).toMatchObject({ failures: [], violations: expected.violations });
    expect(await apply(4, [], [files[0]!.path])).toMatchObject({ failures: [], violations: [] });
  } finally {
    workspace.close();
  }
});

it("keeps direct module attributes separate from attributes of reexported module values", () => {
  const document = policy(`SELECT __path AS path,__start AS start_byte,__end AS end_byte,
    source_name || target_name AS family,'reference' AS issue FROM reference_edges WHERE target_name LIKE '%._effect'`);
  Object.assign(document.clauses[0]!.relational, { references: { language: "python" } });
  const rule = compile(document);
  const files = [
    { path: "pkg/effect.py", language: "python", source: "def _effect(): pass\n" },
    { path: "pkg/bridge.py", language: "python", source: "from pkg import effect\n" },
    { path: "pkg/use.py", language: "python", source: "from pkg.bridge import effect\nfrom pkg import effect as direct\ndef indirect(): return effect._effect()\ndef run(): return direct._effect()\n" },
  ];
  const result = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files }), "report"));
  expect(result.failures).toEqual([]);
  expect(result.violations.map((v: { span: { startLine: number } }) => v.span.startLine)).toEqual([3]);
});

it("reports an authored missing-file location before any source exists and repairs it incrementally", async () => {
  const document = policy(`SELECT 'contract.py' AS path,0 AS start_byte,0 AS end_byte,
    'required' AS family,'missing_contract' AS issue
    WHERE NOT EXISTS(SELECT 1 FROM functions WHERE __path='contract.py' AND name='required')`);
  Object.assign(document.clauses[0]!.relational, { locations: ["contract.py"] });
  expect(codeRuleSchema.safeParse(document).success).toBe(true);
  const rule = compile(document);
  const expected = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files: [] }), "report"));
  expect(expected.failures).toEqual([]);
  expect(expected.violations).toEqual([expect.objectContaining({ path: "contract.py", code: "missing_contract",
    span: { startByte: 0, endByte: 0, startLine: 0, endLine: 0, startColumn: 0, endColumn: 0 } })]);
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(rule);
    const apply = async (revision: number, files: { path: string; source: string; language: string }[], removed: string[] = []) =>
      JSON.parse(await workspace.applyAsync(JSON.stringify({ revision, files, removed }))).evaluation;
    expect(await apply(1, [])).toMatchObject({ failures: [], violations: expected.violations });
    expect(await apply(2, [{ path: "contract.py", language: "python", source: "def required(): pass\n" }])).toMatchObject({ failures: [], violations: [] });
    expect(await apply(3, [], ["contract.py"])).toMatchObject({ failures: [], violations: expected.violations });
  } finally { workspace.close(); }
});

it("refuses undeclared virtual paths and nonzero virtual source offsets", () => {
  for (const [path, start] of [["other.py", 0], ["contract.py", 1]] as const) {
    const document = policy(`SELECT '${path}' AS path,${start} AS start_byte,${start} AS end_byte,'required' AS family,'missing' AS issue`);
    Object.assign(document.clauses[0]!.relational, { locations: ["contract.py"] });
    const result = JSON.parse(compile(document).evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files: [] }), "report"));
    expect(result.violations).toEqual([]);
    expect(result.failures).toEqual([expect.objectContaining({ code: "relational-failed", message: expect.stringContaining("input witness span") })]);
  }
});

it("projects same-node regex positions and uses the selected evidence anchor", async () => {
  const document = policy(`SELECT __path AS path,__start AS start_byte,__end AS end_byte,
    position AS family,'positioned' AS issue,line FROM functions WHERE position=__start AND line IN (1,2)`);
  document.clauses[0]!.relational.inputs.functions = { evidence: { as: "f", query: { ast: { language: "python",
    rule: { kind: "module", regex: "(?P<TOKEN>target)" },
  } }, each: "TOKEN", anchor: "capture.TOKEN", project: { line: "capture.TOKEN.startLine", position: "capture.TOKEN.startByte" } } };
  const rule = compile(document);
  const files = [{ path: "a.py", language: "python", source: "# header\nfirst = 'target'\nsecond = 'target'\n" }];
  const expected = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files }), "report"));
  expect(expected.failures).toEqual([]);
  expect(expected.violations.map((v: { span: { startLine: number; startByte: number } }) => [v.span.startLine, v.span.startByte])).toEqual([[1, 18], [2, 36]]);
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(rule);
    expect(JSON.parse(await workspace.applyAsync(JSON.stringify({ revision: 1, files, removed: [] }))).evaluation).toMatchObject({ failures: [], violations: expected.violations });
  } finally { workspace.close(); }
});

it("joins typed AST nodes to their immediate parents without matching source text", () => {
  const document = policy(`SELECT c.__path AS path,c.__start AS start_byte,c.__end AS end_byte,
    c.name AS family,'owned' AS issue FROM functions c JOIN functions p
    ON p.__path=c.__path AND p.__start=c.parent_start AND p.__end=c.parent_end AND p.kind=c.parent_kind
    WHERE c.kind='identifier' AND c.name='row' AND p.kind='assignment'`);
  document.clauses[0]!.relational.inputs.functions = { evidence: { as: "n", query: { ast: { language: "python", rule: {
    pattern: "$NODE", any: [{ inside: { pattern: "$PARENT", stopBy: "neighbor" } }, { pattern: "$_" }],
  } } }, project: { name: "capture.NODE", kind: "capture.NODE.kind", end: "capture.NODE.endByte",
    parent_start: "capture.PARENT.startByte", parent_end: "capture.PARENT.endByte", parent_kind: "capture.PARENT.kind" } } };
  const result = JSON.parse(compile(document).evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0,
    files: [{ path: "a.py", language: "python", source: "row = [1]\n" }] }), "report"));
  expect(result.failures).toEqual([]); expect(result.violations).toHaveLength(1);
  expect(result.violations[0].span).toMatchObject({ startByte: 0, endByte: 3 });
});
