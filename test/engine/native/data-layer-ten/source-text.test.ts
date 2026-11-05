import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { codeRuleSchema, loadNativeBinding } from "../../../../src/engine/index.js";
it("reads complete non-code and empty files in one SQL statement, preserving absence", () => {
  const authored = { format: "rerail.policy/v1", id: "source-inventory", revision: 1, mode: "log", clauses: [{ id: "files", relational: {
    inputs: { unused: { evidence: { as: "f", query: { ast: { language: "python", rule: { kind: "module" } } }, project: { path: "file.path" } } } },
    locations: ["docs/spec.rst", "empty.txt", "missing.txt"],
    query: "SELECT column1 path,0 start_byte,0 end_byte,1 line,coalesce(source_text(column1),'ABSENT') message,column1 identity FROM (VALUES ('docs/spec.rst'),('empty.txt'),('missing.txt'))", key: ["identity"], line: "line", message: "message",
  }, violation: { code: "SOURCE", message: "Source", anchor: "witness.span" } }] };
  const native = loadNativeBinding();
  const rule = native.CodeRuleDslRuntime.compile(JSON.stringify(codeRuleSchema.parse(authored)), "json", readFileSync("native/src/code_rule_dsl/capabilities.json", "utf8"));
  const files = [{ path: "docs/spec.rst", language: "text", source: "\nDesign\n======\n" }, { path: "empty.txt", language: "text", source: "" }];
  const result = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files }), "report"));
  expect(result.failures).toEqual([]);
  expect(result.violations.map((row: { path: string; message: string }) => [row.path,row.message]).sort()).toEqual([
    ["docs/spec.rst", "\nDesign\n======\n"], ["empty.txt", ""], ["missing.txt", "ABSENT"],
  ]);
});

it("projects regex evidence over text files and narrows a proven span to a diagnostic line", () => {
  const authored = { format: "rerail.policy/v1", id: "text-evidence", revision: 1, mode: "log", clauses: [{ id: "text", relational: {
    inputs: { files: { evidence: { as: "f", query: { regex: { source: "file.content", pattern: "(?s)\\A(?P<SOURCE>.*)\\z" } }, project: { path: "file.path", text: "capture.SOURCE" } } } },
    query: "SELECT __path path,__start start_byte,__end end_byte,3 line,text message FROM files", line: "line", message: "message",
  }, violation: { code: "TEXT", message: "Text", anchor: "witness.span" } }] };
  const native = loadNativeBinding();
  const rule = native.CodeRuleDslRuntime.compile(JSON.stringify(codeRuleSchema.parse(authored)), "json", readFileSync("native/src/code_rule_dsl/capabilities.json", "utf8"));
  const result = JSON.parse(rule.evaluate(JSON.stringify({ revision: 0, files: [{ path: "docs/spec.rst", language: "text", source: "Heading\n=======\nthird\n" }] }), "report"));
  expect(result.failures).toEqual([]);
  expect(result.violations).toMatchObject([{ message: "Heading\n=======\nthird\n", span: { startLine: 2, startByte: 16, endByte: 16 } }]);
});
