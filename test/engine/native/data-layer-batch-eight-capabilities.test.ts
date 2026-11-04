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
