import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { loadNativeBinding } from "../../../src/engine/index.js";

const native = loadNativeBinding();
const caps = readFileSync(new URL("../../../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8");
const source = "class A:\n def f(self):\n  def inner(): return 1\n  return inner()\n";

// Two authored rule shapes supply independent implementations of the same relation.
// Both use the ordinary production engine, without a backend switch.
function compare(predicate: string, empty = false) {
  const roots = empty ? "SELECT __path,id FROM nodes WHERE 0"
    : "SELECT __path,id FROM nodes WHERE kind='function_definition'";
  const stages = [
    { name: "descendants", reachability: { syntax: "nodes", roots } },
    { name: "descendants", query: `WITH RECURSIVE walk(__path,root,node) AS (
        SELECT __path,+id,+id FROM (${roots})
        UNION SELECT w.__path,w.root,n.id FROM walk w JOIN nodes n
        ON n.__path=w.__path AND n.parent=w.node)
        SELECT * FROM walk`, indexes: [["__path", "root", "node"], ["__path", "node", "root"]] },
  ];
  const policy = (stage: typeof stages[number]) => ({
    format: "rerail.policy/v1", id: "completed", revision: 1, mode: "log",
    clauses: [{ id: "rows", relational: {
      inputs: { gate: { evidence: { as: "f", query: { ast: {
        language: "python", syntax: "valid", rule: { kind: "module" },
      } }, project: { path: "file.path" } } } },
      syntax: { name: "nodes", gate: "gate", fields: ["name"] }, stages: [stage],
      query: `SELECT n.__path AS path,n.__start AS start_byte,n.__end AS end_byte,d.root,d.node
        FROM descendants d JOIN nodes n ON n.__path=d.__path AND n.id=d.node WHERE ${predicate}`,
      key: ["root", "node"],
    }, violation: { code: "ROW", message: "Row", anchor: "witness.span" } }],
  });
  const input = JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0,
    files: ["a.py", "b.py"].map(path => ({ path, source, language: "python" })) });
  const [actual, expected] = stages.map(stage => {
    const rule = native.CodeRuleDslRuntime.compile(JSON.stringify(policy(stage)), "json", caps);
    const result = JSON.parse(rule.evaluate(input, "report"));
    expect(result.failures).toEqual([]);
    return result.violations.map((row: unknown) => JSON.stringify(row)).sort();
  });
  expect(actual).toEqual(expected);
  return actual!.length;
}

it("publishes complete forward and reverse rows with file-qualified identities", () => {
  expect(compare("1")).toBeGreaterThan(10);
  expect(compare("d.node IN (SELECT id FROM nodes WHERE kind='identifier')")).toBeGreaterThan(0);
});

it("retains SQLite real, text affinity, null and collation decisions", () => {
  expect(compare("d.root IN (SELECT CAST(id AS REAL) FROM nodes WHERE kind='function_definition')")).toBeGreaterThan(0);
  expect(compare("EXISTS(SELECT 1 FROM (SELECT CAST(id AS TEXT) AS id,__path FROM nodes) t WHERE t.__path=d.__path AND t.id=d.root)")).toBe(0);
  expect(compare("d.root=NULL")).toBe(0);
  expect(compare("d.__path COLLATE NOCASE='A.PY'")).toBeGreaterThan(0);
});

it("completes empty output and deduplicates indexed IN requests", () => {
  expect(compare("1", true)).toBe(0);
  expect(compare("d.root IN (SELECT id FROM nodes UNION ALL SELECT id FROM nodes)")).toBeGreaterThan(0);
});
