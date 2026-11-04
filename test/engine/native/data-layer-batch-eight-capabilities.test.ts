import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { codeRuleSchema, loadNativeBinding } from "../../../src/engine/index.js";

const capabilities = readFileSync(new URL("../../../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8");
const body = "7e783fc807de80e8856ea7d18c4321d18b0ac3c95691c8267538f7c1957c4ced";
const signature = "ac9252cd5bf8aec6165067d9d8554604f302540b556527a083db6cfd46d94fdd";
function policy() {
  return { format: "rerail.policy/v1", id: "python-fingerprints", revision: 1, mode: "log", clauses: [{ id: "fingerprints", relational: {
    inputs: { eligible: { evidence: { as: "row", query: { ast: { language: "python", syntax: "valid", rule: { kind: "module" } } }, project: { path: "file.path" } } } },
    syntax: { name: "nodes", gate: "eligible", fields: ["name"], columns: ["__path", "__start", "__end", "id", "kind", "name_source"] },
    stages: [{ name: "fingerprints", pythonAst: { syntax: "nodes", roots: "SELECT __path,id FROM nodes WHERE kind='function_definition' AND name_source='f'", fields: ["body_sha256", "signature_sha256"] } }],
    query: "SELECT n.__path path,n.__start start_byte,n.__end end_byte,n.id,h.body_sha256,h.signature_sha256 FROM fingerprints h JOIN nodes n ON n.__path=h.__path AND n.id=h.id", key: ["id"],
  }, violation: { code: "FINGERPRINT", message: "Canonical Python fingerprint", anchor: "witness.span" } }] };
}

it("produces exact Python fingerprints for a whole SQL-selected demand batch", () => {
  const authored = policy();
  expect(codeRuleSchema.safeParse(authored).success).toBe(true);
  const native = loadNativeBinding();
  const rule = native.CodeRuleDslRuntime.compile(JSON.stringify(authored), "json", capabilities);
  const files = ["a.py", "b.py"].map(path => ({ path, language: "python", source: "def f(value: int) -> int:\n    return value * 2\n\ndef unrelated():\n    return object()\n" }));
  const result = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files }), "report"));
  expect(result.failures).toEqual([]);
  expect(result.violations).toHaveLength(2);
  expect(result.violations.map((finding: { variables: { row: { body_sha256: string; signature_sha256: string } } }) => finding.variables.row))
    .toEqual([expect.objectContaining({ body_sha256: body, signature_sha256: signature }), expect.objectContaining({ body_sha256: body, signature_sha256: signature })]);
});

it("reads canonical byte columns from selected nodes without a repository source snapshot", () => {
  const authored = policy();
  Object.assign(authored.clauses[0]!.relational.stages[0]!.pythonAst, { fields: ["column"] });
  authored.clauses[0]!.relational.stages[0]!.pythonAst.roots = "SELECT __path,id FROM nodes WHERE kind='function_definition' AND name_source='f'";
  authored.clauses[0]!.relational.query = "SELECT n.__path path,n.__start start_byte,n.__end end_byte,n.id,h.column FROM fingerprints h JOIN nodes n ON n.__path=h.__path AND n.id=h.id";
  const native = loadNativeBinding();
  const rule = native.CodeRuleDslRuntime.compile(JSON.stringify(codeRuleSchema.parse(authored)), "json", capabilities);
  const result = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0,
    files: [{ path: "a.py", language: "python", source: "class Owner:\n    def f(self):\n        return 1\n" }] }), "report"));
  expect(result.failures).toEqual([]);
  expect(result.violations).toHaveLength(1);
  expect(result.violations[0].variables.row.column).toBe("4");
});

it("applies the source byte limit per file in both repository execution paths", async () => {
  const authored = policy();
  const caps = JSON.parse(capabilities); caps.limits.maxSourceBytes = 160;
  const native = loadNativeBinding();
  const rule = native.CodeRuleDslRuntime.compile(JSON.stringify(authored), "json", JSON.stringify(caps));
  const files = ["a.py", "b.py", "c.py"].map(path => ({ path, language: "python",
    source: "def f(value: int) -> int:\n    return value * 2\n# " + "padding".repeat(12) + "\n" }));
  const direct = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files }), "report"));
  expect(direct.failures).toEqual([]); expect(direct.violations).toHaveLength(3);
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(rule);
    const result = JSON.parse(await workspace.applyAsync(JSON.stringify({ revision: 1, files, removed: [] })));
    expect(result.evaluation.failures).toEqual([]); expect(result.evaluation.violations).toHaveLength(3);
  } finally { workspace.close(); }
});

it("identifies incomplete relational input tables and source files", () => {
  const caps = JSON.parse(capabilities); caps.limits.maxWitnesses = 1;
  const authored = { format: "rerail.policy/v1", id: "incomplete-diagnostic", revision: 1, mode: "log", clauses: [{ id: "selected", relational: {
    inputs: { functions: { evidence: { as: "f", query: { ast: { language: "python", rule: { kind: "function_definition" } } }, project: { path: "file.path" } } } },
    query: "SELECT __path path,__start start_byte,__end end_byte FROM functions",
  }, violation: { code: "CHECK", message: "check", anchor: "witness.span" } }] };
  const rule = loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(authored), "json", JSON.stringify(caps));
  const result = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0,
    files: [{ path: "a.py", language: "python", source: "def a(): pass\ndef b(): pass\n" }] }), "report"));
  expect(result.failures).toHaveLength(1);
  expect(result.failures[0].message).toContain("functions");
  expect(result.failures[0].message).toContain("a.py");
});

it("returns canonical annotation spelling and Python repr as typed facts", () => {
  const authored = policy();
  Object.assign(authored.clauses[0]!.relational.stages[0]!.pythonAst, {
    roots: "SELECT __path,id FROM nodes WHERE kind='type'", fields: ["unparse", "repr_unparse"],
  });
  authored.clauses[0]!.relational.query = "SELECT n.__path path,n.__start start_byte,n.__end end_byte,n.id,h.unparse,h.repr_unparse FROM fingerprints h JOIN nodes n ON n.__path=h.__path AND n.id=h.id";
  const rule = loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(codeRuleSchema.parse(authored)), "json", capabilities);
  const result = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0,
    files: [{ path: "a.py", language: "python", source: 'def f(cache: "OptimizerRunCache"):\n    return cache\n' }] }), "report"));
  expect(result.failures).toEqual([]);
  expect(result.violations).toHaveLength(1);
  expect(result.violations[0].variables.row).toMatchObject({ unparse: "'OptimizerRunCache'", repr_unparse: '"\'OptimizerRunCache\'"' });
});
