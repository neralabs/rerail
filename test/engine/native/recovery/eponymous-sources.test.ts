import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { loadNativeBinding } from "../../../../src/engine/index.js";

it("reuses readers across empty, repeated empty and populated syntax domains", () => {
  const native = loadNativeBinding();
  const clauses = [false, false, true].map((selected, index) => ({
    id: "domain-" + index,
    relational: {
      locations: ["a.py"],
      inputs: { gate: { evidence: { as: "g", query: { ast: { language: "python", syntax: "valid",
        rule: selected ? { kind: "module" } : { kind: "function_definition" },
      } }, project: { path: "file.path" } } } },
      syntax: { name: "nodes", gate: "gate", fields: ["name"], edges: "edges", ownership: "ownership", owners: { fn: { kind: "function_definition" } } },
      query: "SELECT 'a.py' path,0 start_byte,0 end_byte,count(*) AS count FROM nodes",
    },
    violation: { code: "COUNT", message: "Count", anchor: "witness.span" },
  }));
  const rule = native.CodeRuleDslRuntime.compile(JSON.stringify({
    format: "rerail.policy/v1", id: "empty-domains", revision: 1, mode: "log", clauses,
  }), "json", readFileSync("native/src/code_rule_dsl/capabilities.json", "utf8"));
  const result = JSON.parse(rule.evaluate(JSON.stringify({
    format: "rerail.repository-snapshot/v1", revision: 1,
    files: [{ path: "a.py", language: "python", source: "pass\n" }],
  }), "report"));
  expect(result.failures).toEqual([]);
  expect(result.violations).toHaveLength(3);
  const counts = Object.fromEntries(result.violations.map((row: { clauseId: string; variables: { row: { count: number } } }) => [row.clauseId, row.variables.row.count]));
  expect(counts["domain-0"]).toBe(0);
  expect(counts["domain-1"]).toBe(0);
  expect(counts["domain-2"]).toBeGreaterThan(0);
});
