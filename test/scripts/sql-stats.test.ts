import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { loadNativeBinding } from "../../src/engine/index.js";
import { statisticsPolicy } from "../../scripts/profiling/sql-stats.js";

const document = () => ({ format: "rerail.policy/v1", id: "sql-statistics-probe", revision: 1, mode: "log",
  clauses: [{ id: "diagnostics", relational: { partitionBy: "file", inputs: { functions: { evidence: { as: "f",
    query: { ast: { language: "python", rule: { kind: "function_definition", has: { field: "name", pattern: "$NAME" } } } },
    project: { name: "capture.NAME" } } } },
    stages: [{ name: "facts", query: "SELECT * FROM functions UNION ALL SELECT * FROM functions", indexes: [["name"]] }],
    query: "SELECT __path AS path,__start AS start_byte,__end AS end_byte,name FROM facts WHERE name='missing'", key: ["name"] },
    violation: { code: "MISSING", message: "Missing", anchor: "witness.span" } }],
});

it("counts input and materialized rows without changing their SQL or the original policy", () => {
  const original = document(), before = structuredClone(original);
  const probe = statisticsPolicy(original, { diagnostics: [{ relation: "facts", metric: "names", query: "SELECT count(DISTINCT name) FROM facts" }] });
  expect(original).toEqual(before);
  expect(probe).toMatchObject({ clauses: [{ relational: { inputs: original.clauses[0]!.relational.inputs,
    stages: original.clauses[0]!.relational.stages } }] });
  const native = loadNativeBinding();
  const caps = readFileSync(new URL("../../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8");
  const compile = (value: unknown) => native.CodeRuleDslRuntime.compile(JSON.stringify(value), "json", caps);
  const snapshot = JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 1,
    files: [{ path: "a.py", language: "python", source: "def read(): pass\ndef write(): pass\n" }] });
  expect(JSON.parse(compile(original).evaluate(snapshot, "report")).violations).toEqual([]);
  const result = JSON.parse(compile(probe).evaluate(snapshot, "report"));
  expect(result.failures).toEqual([]);
  expect(result.violations.map((row: { variables: { row: unknown } }) => row.variables.row)).toEqual(expect.arrayContaining([
    expect.objectContaining({ relation: "functions", metric: "rows", value: 2 }),
    expect.objectContaining({ relation: "facts", metric: "rows", value: 4 }),
    expect.objectContaining({ relation: "facts", metric: "names", value: 2 }),
  ]));
  expect(result.violations).toHaveLength(3);
});

it("rejects unknown custom metric relations", () => {
  expect(() => statisticsPolicy(document(), { diagnostics: [{ relation: "missing", metric: "rows", query: "SELECT 1" }] })).toThrow("Unknown metric relation");
});
