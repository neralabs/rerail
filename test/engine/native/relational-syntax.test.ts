import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it } from "vitest";
import { codeRuleSchema, loadNativeBinding } from "../../../src/engine/index.js";

const native = loadNativeBinding();
const caps = readFileSync(new URL("../../../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8");
function policy() {
  return { format: "rerail.policy/v1", id: "syntax", revision: 1, mode: "log", clauses: [{ id: "nodes", relational: {
    partitionBy: "file", inputs: { eligible: { evidence: { as: "file", query: { ast: { language: "python", syntax: "valid", rule: { kind: "module" } } }, project: { path: "file.path" } } } },
    syntax: { name: "nodes", gate: "eligible", fields: ["name", "value", "body"], edges: "edges", ownership: "ownership", owners: { fn: { kind: "function_definition" }, cls: { kind: "class_definition" }, top_fn: { kind: "function_definition", outermost: true } } },
    stages: [{ name: "descendants", reachability: { roots: "SELECT __path,id FROM nodes WHERE kind='function_definition'", syntax: "nodes" } }],
    query: "SELECT n.__path AS path,n.__start AS start_byte,n.__end AS end_byte,n.text,n.id,n.name_id,o.fn,(SELECT count(*) FROM descendants d WHERE d.__path=n.__path AND d.root=n.id) AS descendants FROM nodes n JOIN ownership o ON o.__path=n.__path AND o.id=n.id WHERE n.kind='function_definition'",
    key: ["id"],
  }, violation: { code: "NODE", message: "Node", anchor: "witness.span" } }] };
}
const compile = (value: unknown, capabilities = caps) => native.CodeRuleDslRuntime.compile(JSON.stringify(value), "json", capabilities);
const files = [{ path: "a.py", language: "python", source: "class A:\n def f(self):\n  def inner(): return 1\n  return inner()\n" }];
const evaluate = (rule: ReturnType<typeof compile>, current = files) => JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files: current }), "report"));

it("uses native identities, shared owners and direct descendants in SQL", () => {
  const value = policy();
  expect(codeRuleSchema.safeParse(value).success).toBe(true);
  const result = evaluate(compile(value));
  expect(result.failures).toEqual([]);
  expect(result.violations).toHaveLength(2);
  const rows = result.violations.map((v: { variables: { row: { id: number; fn: number; name_id: number; descendants: number } } }) => v.variables.row);
  expect(rows.every((r: typeof rows[number]) => Number.isSafeInteger(r.id) && r.id === r.fn && r.name_id > r.id && r.descendants > 1)).toBe(true);
});

it("exposes exclusive preorder bounds and applies expansion guards to syntax roots", () => {
  const value=policy();
  value.clauses[0]!.relational.stages[0]!.reachability.roots="SELECT __path,id,name_source!='inner' AS expand FROM nodes WHERE kind='function_definition'";
  value.clauses[0]!.relational.query="SELECT n.__path AS path,n.__start AS start_byte,n.__end AS end_byte,n.id,n.subtree_end,(SELECT count(*) FROM descendants d WHERE d.__path=n.__path AND d.root=n.id) AS count,(SELECT count(*) FROM nodes child WHERE child.__path=n.__path AND child.id>=n.id AND child.id<n.subtree_end) AS interval_count FROM nodes n WHERE n.kind='function_definition'";
  const result=evaluate(compile(value));
  expect(result.failures).toEqual([]);
  expect(result.violations).toHaveLength(2);
  const rows=result.violations.map((v:{variables:{row:{id:number;subtree_end:number;count:number;interval_count:number}}})=>v.variables.row);
  expect(rows.every((r:typeof rows[number])=>r.subtree_end>r.id)).toBe(true);
  expect(rows.filter((r:typeof rows[number])=>r.count===1)).toHaveLength(1);
  expect(rows.some((r:typeof rows[number])=>r.count>1&&r.count===r.interval_count)).toBe(true);
});

it("preserves source gates, scope, same-length edits, restoration and removal", async () => {
  const value = policy();
  Object.assign(value, { scope: { include: ["a.py"], exclude: [] } });
  const rule = compile(value), workspace = new native.CodeRuleDslWorkspace();
  workspace.addCodeRule(rule);
  try {
    const compare = async (revision: number, changed: typeof files, current: typeof files, removed: string[] = []) => {
      const actual = JSON.parse(await workspace.applyAsync(JSON.stringify({ revision, files: changed, removed }))).evaluation;
      const expected = evaluate(rule, current);
      expect(actual.failures).toEqual([]);
      expect(actual.violations).toEqual(expected.violations);
    };
    const outside = { ...files[0]!, path: "outside.py" };
    await compare(1, [...files, outside], [...files, outside]);
    const edited = { ...files[0]!, source: files[0]!.source.replace("inner", "other") };
    await compare(2, [edited], [edited, outside]);
    await compare(3, files, [...files, outside]);
    await compare(4, [], [outside], ["a.py"]);
  } finally { workspace.close(); }
});

it("does not expose syntax from files without the input gate", () => {
  const value = policy();
  Object.assign(value.clauses[0]!.relational.inputs.eligible.evidence.query.ast.rule, { regex: "ELIGIBLE" });
  expect(evaluate(compile(value)).violations).toEqual([]);
});

it("keeps repository identities file-qualified and supports batched owner lookups", () => {
  const value = policy();
  value.clauses[0]!.relational.partitionBy = "repository";
  value.clauses[0]!.relational.query = "SELECT n.__path AS path,n.__start AS start_byte,n.__end AS end_byte,n.text,n.id,o.fn,o.top_fn,o.cls FROM nodes n JOIN ownership o ON o.__path=n.__path AND o.id=n.id WHERE n.kind='function_definition' AND o.top_fn IN (SELECT id FROM nodes WHERE kind='function_definition')";
  const result = evaluate(compile(value), [...files, { ...files[0]!, path: "b.py" }]);
  expect(result.failures).toEqual([]);
  expect(result.violations).toHaveLength(4);
  expect(new Set(result.violations.map((v: { path: string }) => v.path))).toEqual(new Set(["a.py", "b.py"]));
});

it("keeps SQLite null, affinity and collation checks for native lookup predicates", () => {
  const value = policy();
  value.clauses[0]!.relational.query = "SELECT n.__path AS path,n.__start AS start_byte,n.__end AS end_byte,n.text,n.id FROM nodes n WHERE n.kind COLLATE NOCASE='FUNCTION_DEFINITION' AND n.id IN (SELECT CAST(id AS REAL) FROM nodes WHERE kind='function_definition') AND n.id NOT IN (-1)";
  const result = evaluate(compile(value));
  expect(result.failures).toEqual([]);
  expect(result.violations).toHaveLength(2);
});

it("uses bounded start/end ranges within a function and retains zero-width boundaries", () => {
  const value = policy();
  value.clauses[0]!.relational.query = "SELECT n.__path AS path,n.__start AS start_byte,n.__end AS end_byte,n.id,n.text FROM nodes owner JOIN nodes n ON n.__path=owner.__path AND n.__start>=owner.__start AND n.__end<=owner.__end WHERE owner.kind='function_definition' AND owner.name_source='inner' AND n.kind='integer'";
  const result = evaluate(compile(value));
  expect(result.failures).toEqual([]);
  expect(result.violations).toHaveLength(1);
  expect(result.violations[0].variables.row.text).toBe("1");
});

it("normalizes typed syntax identities through indexed SQL within the work allowance", () => {
  const value = policy(), graph = value.clauses[0]!.relational.stages[0]!;
  Object.assign(graph, { reachability: { roots: "SELECT __path,CAST(id AS INTEGER) AS id FROM nodes WHERE kind='function_definition'", edges: "SELECT __path,CAST(src AS INTEGER) AS src,CAST(dst AS INTEGER) AS dst FROM edges", graph: "forest" } });
  const bounded = JSON.parse(caps);
  bounded.limits.maxVisitedNodes = 100_000;
  bounded.limits.maxVisitsPerNode = 0;
  const current = [{ path: "a.py", language: "python", source: Array.from({ length: 350 }, (_, i) => `def f${i}(): return ${i}\n`).join("") }];
  const result = evaluate(compile(value, JSON.stringify(bounded)), current);
  expect(result.failures).toEqual([]);
  expect(result.violations).toHaveLength(350);
});

it("keeps direct descendant joins to indexed SQL facts within the work allowance", () => {
  const value = policy(), plan = value.clauses[0]!.relational;
  Object.assign(plan, { stages: [...plan.stages, { name: "names", query: "SELECT __path,id,__start,__end FROM nodes WHERE kind='integer'", indexes: [["__path", "id"]] }] });
  plan.query = "SELECT n.__path AS path,n.__start AS start_byte,n.__end AS end_byte,n.id FROM descendants d CROSS JOIN names n ON n.__path=d.__path AND n.id=d.node";
  const bounded = JSON.parse(caps);
  bounded.limits.maxVisitedNodes = 15_000;
  bounded.limits.maxVisitsPerNode = 0;
  const current = [{ path: "a.py", language: "python", source: Array.from({ length: 350 }, (_, i) => `def f${i}(): return ${i}\n`).join("") }];
  const result = evaluate(compile(value, JSON.stringify(bounded)), current);
  expect(result.failures).toEqual([]);
  expect(result.violations).toHaveLength(350);
});

it("preserves zero-based evidence lines", () => {
  const value = policy();
  value.clauses[0]!.relational.query = "SELECT __path AS path,__start AS start_byte,__end AS end_byte,id,line FROM nodes WHERE kind='function_definition'";
  const result = evaluate(compile(value));
  expect(result.failures).toEqual([]);
  expect(result.violations.every((v: { span: { startLine: number }; variables: { row: { line: number } } }) => v.span.startLine === v.variables.row.line)).toBe(true);
});

it("matches evidence metadata and source fields across nested Python shapes", () => {
  const value = policy(), plan = value.clauses[0]!.relational;
  plan.stages = [];
  plan.query = "SELECT __path AS path,__start AS start_byte,__end AS end_byte,kind,line,parent_start,parent_end,parent_kind,name_source,value_source,body_source FROM nodes WHERE kind IN ('class_definition','function_definition','assignment','keyword_argument','identifier','attribute','call','for_statement','type')";
  plan.key = ["kind", "start_byte", "end_byte"];
  const optional = (rule: unknown) => ({ any: [rule, { pattern: "$_" }] });
  const { syntax: _syntax, ...rest } = plan;
  const legacy = { ...value, clauses: [{ ...value.clauses[0]!, relational: { ...rest,
    inputs: { raw: { evidence: { as: "row", query: { ast: { language: "python", syntax: "valid", rule: { all: [
      { pattern: "$NODE" }, optional({ inside: { pattern: "$PARENT", stopBy: "neighbor" } }),
      ...["name", "value", "body"].map(field => optional({ has: { field, pattern: "$" + field.toUpperCase(), stopBy: "neighbor" } })),
    ] } } }, project: { kind: "capture.NODE.kind", line: "capture.NODE.startLine", parent_start: "capture.PARENT.startByte", parent_end: "capture.PARENT.endByte", parent_kind: "capture.PARENT.kind", name_source: "capture.NAME", value_source: "capture.VALUE", body_source: "capture.BODY" } } } },
    query: plan.query.replace("FROM nodes", "FROM raw"),
  } }] };
  const current = [{ path: "nested.py", language: "python", source: "# Unicode π\n@decorate(flag=True)\nclass A:\n def f(self, value: list[int]):\n  result = [x.attr for x in value]\n  return call(name='☃', value=result)\n" }];
  const actual = evaluate(compile(value), current), expected = evaluate(compile(legacy), current);
  expect(actual.failures).toEqual([]);
  expect(expected.failures).toEqual([]);
  expect(actual.violations.map((v: { variables: { row: unknown } }) => v.variables.row)).toEqual(expected.violations.map((v: { variables: { row: unknown } }) => v.variables.row));
});

it("retains ignored nodes as facts and suppresses findings at their anchors", () => {
  const current = [{ ...files[0]!, source: files[0]!.source.replace("  def inner", "  # rerail-ignore syntax\n  def inner") }];
  const result = evaluate(compile(policy()), current);
  expect(result.failures).toEqual([]);
  expect(result.violations).toHaveLength(1);
  expect(result.violations[0].variables.row.text).toContain("inner");
});

it("restores shared metadata after document eviction and refuses stale source values", async () => {
  const root = mkdtempSync(join(tmpdir(), "rerail-syntax-"));
  const current = [{ ...files[0]!, source: "def before(): return 'π'\n" }];
  writeFileSync(join(root, "a.py"), current[0]!.source);
  execFileSync("git", ["init", "--quiet"], { cwd: root });
  execFileSync("git", ["add", "a.py"], { cwd: root });
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "source fixture"], { cwd: root });
  const rule = compile(policy()), workspace = new native.CodeRuleDslWorkspace();
  try {
    await workspace.enableStorageAsync(root, join(root, "results.sqlite"));
    workspace.setDocumentResidencyLimit(0);
    workspace.addCodeRule(rule);
    await workspace.applyAsync(JSON.stringify({ revision: 1, files: current, removed: [] }));
    await workspace.completeStorageRevisionAsync();
    expect(workspace.evictInactive()).toBe(true);
    const edited = [{ ...current[0]!, source: current[0]!.source.replace("before", "after_") }];
    const actual = JSON.parse(await workspace.applyAsync(JSON.stringify({ revision: 2, files: edited, removed: [] }))).evaluation;
    expect(actual.failures).toEqual([]);
    expect(actual.violations).toEqual(evaluate(rule, edited).violations);
  } finally { workspace.close(); rmSync(root, { recursive: true, force: true }); }
});

it("rejects unavailable gates and fields, conflicting names and incompatible graph declarations", () => {
  for (const shape of [{ gate: "missing" }, { fields: ["no_such_field"] }, { name: "eligible" }, { columns: ["id", "no_such_column"] }]) {
    const value = policy(); Object.assign(value.clauses[0]!.relational.syntax, shape);
    expect(() => compile(value)).toThrow();
  }
  const value = policy();
  Object.assign(value.clauses[0]!.relational.stages[0]!.reachability, { edges: "SELECT __path,src,dst FROM edges" });
  expect(() => compile(value)).toThrow();
});

it("matches the SQL forest oracle for every descendant row across files and exclusions", () => {
  const value = policy();
  Object.assign(value.clauses[0]!.relational.syntax, { excludeKinds: ["comment", "string_content"] });
  value.clauses[0]!.relational.query = "SELECT n.__path AS path,n.__start AS start_byte,n.__end AS end_byte,d.root,d.node,n.kind FROM descendants d JOIN nodes n ON n.__path=d.__path AND n.id=d.node";
  value.clauses[0]!.relational.key = ["root", "node"];
  const control = structuredClone(value);
  Object.assign(control.clauses[0]!.relational.stages[0]!, { reachability: { roots: "SELECT __path,id FROM nodes WHERE kind='function_definition'", edges: "SELECT __path,src,dst FROM edges", graph: "forest" } });
  const current = [...files, { path: "ü.py", language: "python", source: "def π():\n # comment\n def child(): return '文字'\n return child()\n" }];
  const actual = evaluate(compile(value), current), expected = evaluate(compile(control), current);
  expect(actual.failures).toEqual([]); expect(expected.failures).toEqual([]);
  expect(actual.violations.length).toBeGreaterThan(10);
  expect(actual.violations).toEqual(expected.violations);
});

it("keeps a failed producer separate from another completed relation", () => {
  const value = policy(), bad = structuredClone(value.clauses[0]!);
  bad.id = "bad";
  bad.relational.stages[0]!.reachability.roots = "SELECT __path,999999 AS id FROM nodes WHERE kind='module'";
  value.clauses.push(bad);
  const result = evaluate(compile(value));
  expect(result.failures).toHaveLength(1);
  expect(result.failures[0].clauseId).toBe("bad");
  expect(result.violations).toHaveLength(2);
  expect(result.violations.every((v: { clauseId: string }) => v.clauseId === "nodes")).toBe(true);
});

it("publishes successful empty relations before SQL reasons about absence", () => {
  const value = policy();
  value.clauses[0]!.relational.stages[0]!.reachability.roots = "SELECT __path,id FROM nodes WHERE 0";
  value.clauses[0]!.relational.query = "SELECT __path AS path,__start AS start_byte,__end AS end_byte,id FROM nodes n WHERE kind='function_definition' AND NOT EXISTS(SELECT 1 FROM descendants d WHERE d.__path=n.__path AND d.root=n.id)";
  const result = evaluate(compile(value));
  expect(result.failures).toEqual([]); expect(result.violations).toHaveLength(2);
});
