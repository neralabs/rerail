import { expect, it } from "vitest";
import { codeRuleSchema, loadNativeBinding } from "../../../../src/engine/index.js";
import { readFileSync } from "node:fs";

const capabilities = readFileSync("native/src/code_rule_dsl/capabilities.json", "utf8");
it("publishes canonical argument containers with their actual function or lambda owner", () => {
  const source = "def outer(value: int):\n    return lambda untyped: untyped\n";
  const response = JSON.parse(loadNativeBinding().CodeRuleDslRuntime.pythonAstBatch(JSON.stringify([{ source, requests: [{ start: 0, end: Buffer.byteLength(source), kind: "module", fields: ["ast_nodes"], columns: ["ast_kind", "ast_preorder", "ast_parent"] }] }])));
  const columns = JSON.parse(response.documents[0][0].ast_nodes);
  const containers = columns.ast_kind.flatMap((kind: string, i: number) => kind === "arguments" ? [i] : []);
  expect(containers).toHaveLength(2);
  expect(containers.map((i: number) => columns.ast_kind[columns.ast_preorder.indexOf(columns.ast_parent[i])])).toEqual(["FunctionDef", "Lambda"]);
});

it("parses invalid source in one native relation without requiring a valid CST", () => {
  const authored = { format: "rerail.policy/v1", id: "source-parser", revision: 1, mode: "log", clauses: [{ id: "parser", relational: {
    inputs: { invalid: { evidence: { as: "f", query: { ast: { language: "python", syntax: "invalid", rule: { kind: "module" } } }, project: { path: "file.path" } } } },
    stages: [{ name: "parsed", pythonAst: { source: "invalid", roots: "SELECT __path,source_text(__path,0,__end) AS source FROM invalid", fields: ["syntax_error"] } }],
    query: "SELECT i.__path path,0 start_byte,0 end_byte,p.syntax_error message,i.__path identity FROM invalid i JOIN parsed p ON p.__path=i.__path", key: ["identity"], message: "message",
  }, violation: { code: "PARSE", message: "Parse error", anchor: "witness.span" } }] };
  const rule = loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(codeRuleSchema.parse(authored)), "json", capabilities);
  const result = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files: [{ path: "bad.py", source: "\n\ndef broken(\n", language: "python" }] }), "report"));
  expect(result.failures).toEqual([]);
  expect(result.violations).toHaveLength(1);
  expect(JSON.parse(result.violations[0].message)).toMatchObject({ msg: "'(' was never closed", lineno: 3 });
});

it("selects a diagnostic line only within its admitted witness span", () => {
  const relational = { inputs: { f: { evidence: { as: "f", query: { ast: { language: "python", rule: { kind: "module" } } }, project: { path: "file.path" } } } },
    syntax: { name: "cst", gate: "f", columns: ["__path", "id", "kind", "__start", "__end"] },
    query: "SELECT __path path,__start start_byte,__end end_byte,3 line FROM cst WHERE kind='module'", line: "line" };
  const authored = { format: "rerail.policy/v1", id: "line-anchor", revision: 1, mode: "log", clauses: [{ id: "line", relational,
    violation: { code: "LINE", message: "Line diagnostic", anchor: "witness.span" } }] };
  const snapshot = JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files: [{ path: "a.py", source: "pass\n\n# third\n", language: "python" }] });
  function evaluate(query: string) {
    const program = { ...authored, clauses: [{ ...authored.clauses[0], relational: { ...relational, query } }] };
    const rule = loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(codeRuleSchema.parse(program)), "json", capabilities);
    return JSON.parse(rule.evaluate(snapshot, "report"));
  }
  expect(evaluate(relational.query)).toMatchObject({ failures: [], violations: [{ span: { startLine: 2, startByte: 6, endByte: 6 } }] });
  for (const line of [0, 99, -1, 1.5, "'3'"]) {
    const result = evaluate(relational.query.replace("3 line", `${line} line`));
    expect(result.violations).toEqual([]); expect(result.failures).toHaveLength(1);
  }
});
