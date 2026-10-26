import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { loadNativeBinding } from "../../../../src/engine/index.js";
const native = loadNativeBinding();
const caps = readFileSync(new URL("../../../../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8");
const fields = ["name", "body", "parameters", "value"];
const excluded = ["comment", "string_start", "string_content", "string_end", "escape_sequence", "line_continuation"];
const optional = (r: unknown) => ({ any: [r, { pattern: "$_" }] });
const snapshot = { format: "rerail.repository-snapshot/v1", revision: 1, files: ["a.py", "b.py"].map(path => ({ path, language: "python", source: `# excluded comment
@decorate
class Ω:
    def outer(self, value=None):
        text = "café\\n"
        def inner(x):
            return x + 1
        return inner(value)

result = [v for v in (1, 2)]
` })) };
function policy(tape: boolean, expand: boolean) {
  const project: Record<string, string> = { kind: "capture.NODE.kind", text: "capture.NODE", line: "capture.NODE.startLine", parent_start: "capture.PARENT.startByte", parent_end: "capture.PARENT.endByte", parent_kind: "capture.PARENT.kind", ancestor_start: "capture.PARENT.startByte", ancestor_end: "capture.PARENT.endByte", ancestor_kind: "capture.PARENT.kind" };
  for (const field of fields) for (const [suffix, value] of Object.entries({ start: "startByte", end: "endByte", kind: "kind", source: "" })) project[`${field}_${suffix}`] = `capture.${field.toUpperCase()}${value ? "." + value : ""}`;
  const ast = { as: "n", query: { ast: { language: "python", syntax: "valid", rule: { all: [{ pattern: "$NODE" }, optional({ inside: { pattern: "$PARENT", stopBy: "neighbor" } }), ...fields.map(field => optional({ has: { pattern: "$" + field.toUpperCase(), field, stopBy: "neighbor" } })), { not: { any: excluded.map(kind => ({ kind })) } }] } } }, project };
  const sql = `SELECT n.__path path,n.__start start_byte,n.__end end_byte,n.kind,n.line,n.ordinal,n.text,
    p.kind parent_kind,p.__start parent_start,p.__end parent_end,
    f.__start owner_start,c.__start class_start,t.__start outer_start,
    n.name_start,n.name_end,n.name_kind,n.name_source,n.value_start,n.value_end,n.value_kind,n.value_source,
    (SELECT count(*) FROM edges e WHERE e.__path=n.__path AND e.src=n.id) children,
    (SELECT count(*) FROM descendants d WHERE d.__path=n.__path AND d.root=n.id) descendants
    FROM nodes n LEFT JOIN nodes p ON p.__path=n.__path AND p.id=n.parent
    LEFT JOIN ownership o ON o.__path=n.__path AND o.id=n.id
    LEFT JOIN nodes f ON f.__path=n.__path AND f.id=o.fn
    LEFT JOIN nodes c ON c.__path=n.__path AND c.id=o.cls
    LEFT JOIN nodes t ON t.__path=n.__path AND t.id=o.top_fn`;
  const roots = "SELECT __path,id" + (expand ? ",kind NOT IN ('module','block','expression_statement','argument_list','decorated_definition','type') AS expand" : "") + " FROM nodes";
  const stages = tape ? [] : [
    { name: "nodes", query: "SELECT *,kind||':'||__start||':'||__end id,parent_kind||':'||parent_start||':'||parent_end parent,ancestor_kind||':'||ancestor_start||':'||ancestor_end edge_parent,row_number() OVER(PARTITION BY __path,parent_start,parent_end,parent_kind ORDER BY __start,__end,kind)-1 ordinal FROM raw_nodes" },
    { name: "edges", query: "SELECT __path,edge_parent src,id dst,ordinal FROM nodes WHERE edge_parent IS NOT NULL" },
    { name: "ownership", query: "WITH RECURSIVE owned(__path,id,fn,cls,top_fn) AS (SELECT n.__path,n.id,CASE WHEN n.kind='function_definition' THEN n.id END,CASE WHEN n.kind='class_definition' THEN n.id END,CASE WHEN n.kind='function_definition' THEN n.id END FROM nodes n LEFT JOIN nodes p ON p.__path=n.__path AND p.id=n.edge_parent WHERE p.id IS NULL UNION ALL SELECT n.__path,n.id,CASE WHEN n.kind='function_definition' THEN n.id ELSE o.fn END,CASE WHEN n.kind='class_definition' THEN n.id ELSE o.cls END,CASE WHEN o.top_fn IS NULL AND n.kind='function_definition' THEN n.id ELSE o.top_fn END FROM owned o JOIN edges e ON e.__path=o.__path AND e.src=o.id JOIN nodes n ON n.__path=e.__path AND n.id=e.dst) SELECT * FROM owned" },
  ];
  const plan = { partitionBy: "repository", inputs: tape ? { gate: { evidence: { as: "g", query: { ast: { language: "python", syntax: "valid", rule: { kind: "module" } } }, project: { path: "file.path" } } } } : { raw_nodes: { evidence: ast } },
    ...(tape ? { syntax: { name: "nodes", gate: "gate", fields, excludeKinds: excluded, edges: "edges", ownership: "ownership", owners: { fn: { kind: "function_definition" }, cls: { kind: "class_definition" }, top_fn: { kind: "function_definition", outermost: true } } } } : {}),
    stages: [...stages, { name: "descendants", reachability: tape ? { roots, syntax: "nodes" } : { roots, edges: "SELECT __path,src,dst FROM edges", graph: "forest" } }], query: sql, key: ["kind"] };
  return { format: "rerail.policy/v1", id: "domain", revision: 1, mode: "log", clauses: [{ id: "rows", relational: plan, violation: { code: "ROW", message: "Row", anchor: "witness.span" } }] };
}
it("preserves node, field, parent, owner, ordinal and descendant bags over both files", () => {
  for (const expand of [false, true]) {
    const run = (tape: boolean) => {
      const compiled = native.CodeRuleDslRuntime.compile(JSON.stringify(policy(tape, expand)), "json", caps);
      const result = JSON.parse(compiled.evaluate(JSON.stringify(snapshot), "report"));
      expect(result.failures).toEqual([]);
      return result.violations.map((v: { variables: { row: unknown } }) => JSON.stringify(v.variables.row)).sort();
    };
    const legacy = run(false), tape = run(true);
    expect(legacy.length).toBeGreaterThan(70); expect(tape).toEqual(legacy);
  }
});

it("collects conditional import declarations without their unused identifier payloads", () => {
  const production = JSON.parse(readFileSync(new URL("../../../../packs/data-layer/candidates/rules/data-layer-serverplan-quality.json", import.meta.url), "utf8"));
  const plan = production.clauses.find((c: { id: string }) => c.id === "quality-conditional-1").relational;
  const inputs = plan.inputs as Record<string, unknown>;
  const query = Object.keys(inputs).map(name => `SELECT __path path,__start start_byte,__end end_byte,kind FROM ${name}`).join(" UNION ALL ");
  production.clauses = [production.clauses[0], { id: "import-demand", relational: { inputs, query, key: ["kind"] },
    violation: { code: "NODE", message: "Selected syntax", anchor: "witness.span" } }];
  const compiled = native.CodeRuleDslRuntime.compile(JSON.stringify(production), "json", caps);
  const source = Array.from({ length: 100 }, (_, i) => `from alpha.beta${i} import charlie as delta${i}\n`).join("");
  const result = JSON.parse(compiled.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 1,
    files: [{ path: "a.py", language: "python", source }] }), "report"));
  expect(result.failures).toEqual([]);
  const kinds = result.violations.map((v: { variables: { row: { kind: string } } }) => v.variables.row.kind);
  expect(kinds.filter((kind: string) => kind === "identifier")).toHaveLength(0);
  expect(kinds.filter((kind: string) => kind === "import_from_statement")).toHaveLength(100);
  expect(kinds.filter((kind: string) => kind === "aliased_import")).toHaveLength(100);
  expect(kinds.filter((kind: string) => kind === "dotted_name")).toHaveLength(100);
});
