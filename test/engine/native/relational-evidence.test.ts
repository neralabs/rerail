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
