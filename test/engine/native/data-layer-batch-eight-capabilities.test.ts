import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { codeRuleSchema, loadNativeBinding } from "../../../src/engine/index.js";

const capabilities = readFileSync(new URL("../../../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8");
const body = "7e783fc807de80e8856ea7d18c4321d18b0ac3c95691c8267538f7c1957c4ced";
const signature = "ac9252cd5bf8aec6165067d9d8554604f302540b556527a083db6cfd46d94fdd";

it("returns canonical syntax errors for a complete batch without executing source", () => {
  const sources = ["def invalid(\n", "raise RuntimeError('never execute')\n"];
  const documents = sources.map(source => ({ source, requests: [{ start: 0, end: Buffer.byteLength(source), kind: "module", fields: ["syntax_error"] }] }));
  const response = JSON.parse(loadNativeBinding().CodeRuleDslRuntime.pythonAstBatch(JSON.stringify(documents)));
  expect(response.parses).toBe(2); expect(response.module_walks).toBe(0);
  expect(JSON.parse(response.documents[0][0].syntax_error)).toEqual({ msg: "'(' was never closed", lineno: 1,
    offset: 12, text: "def invalid(\n", end_lineno: 1, end_offset: 0 });
  expect(JSON.parse(response.documents[1][0].syntax_error)).toBeNull();
  documents[0]!.requests[0]!.fields.push("dump");
  expect(() => loadNativeBinding().CodeRuleDslRuntime.pythonAstBatch(JSON.stringify(documents))).toThrow(/CPython AST batch failed/);
});

it("shares ordered graph memo entries across demanded roots and isolates their scopes", () => {
  const authored = { format: "rerail.policy/v1", id: "ordered-graph", revision: 1, mode: "log", clauses: [{
    id: "diagnostic", relational: {
      inputs: { empty: { evidence: { as: "f", query: { ast: { language: "python", rule: { kind: "module" } } }, project: { path: "file.path" } } } },
      locations: ["metadata"], stages: [{ name: "matches", reachability: {
        selection: "memoized-first", roots: "SELECT 'one' __path,'a' id,0 ordinal UNION ALL SELECT 'one','b',1 UNION ALL SELECT 'two','b',0",
        edges: "SELECT 'one' __path,'a' src,'b' dst,0 ordinal,90 location UNION ALL SELECT 'one','a','sink',1,12 UNION ALL SELECT 'one','b','a',0,11 UNION ALL SELECT 'two','b','sink',0,7",
        sinks: "SELECT NULL __path,'sink' id,100 location",
      } }, { name: "consumed", query: "SELECT * FROM matches" }], query: "SELECT 'metadata' path,0 start_byte,0 end_byte,__path||':'||root identity,__path scope,root,node,location FROM consumed", key: ["identity"],
    }, violation: { code: "GRAPH", message: "Ordered match", anchor: "witness.span" },
  }] };
  const rule = loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(codeRuleSchema.parse(authored)), "json", capabilities);
  const result = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files: [] }), "report"));
  expect(result.failures).toEqual([]);
  expect(result.violations.map((row: { variables: { row: unknown } }) => row.variables.row)).toEqual([
    { path: "metadata", start_byte: 0, end_byte: 0, identity: "one:a", scope: "one", root: "a", node: "sink", location: 12 },
    { path: "metadata", start_byte: 0, end_byte: 0, identity: "two:b", scope: "two", root: "b", node: "sink", location: 7 },
  ]);
});

it("uses the SQL row's exact diagnostic message", () => {
  const authored = { format: "rerail.policy/v1", id: "diagnostic-payload", revision: 1, mode: "log", clauses: [{
    id: "diagnostic", relational: {
      inputs: { files: { evidence: { as: "f", query: { ast: { language: "python", rule: { kind: "module" } } }, project: { path: "file.path" } } } },
      query: "SELECT __path path,__start start_byte,__end end_byte,'Owner '||__path||' reaches an eager helper' detail FROM files",
      message: "detail",
    }, violation: { code: "OWNER", message: "This fallback must not mask a missing payload", anchor: "witness.span" },
  }] };
  const rule = loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(codeRuleSchema.parse(authored)), "json", capabilities);
  const result = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files: [{ path: "owner.py", language: "python", source: "pass\n" }] }), "report"));
  expect(result.failures).toEqual([]);
  expect(result.violations.map((finding: { message: string }) => finding.message)).toEqual(["Owner owner.py reaches an eager helper"]);
});

it("rejects a missing SQL message column instead of silently using generic prose", () => {
  const authored = { format: "rerail.policy/v1", id: "missing-message", revision: 1, mode: "log", clauses: [{
    id: "diagnostic", relational: {
      inputs: { files: { evidence: { as: "f", query: { ast: { language: "python", rule: { kind: "module" } } }, project: { path: "file.path" } } } },
      query: "SELECT __path path,__start start_byte,__end end_byte FROM files", message: "missing",
    }, violation: { code: "OWNER", message: "Fallback", anchor: "witness.span" },
  }] };
  expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(authored), "json", capabilities)).toThrow(/unknown output column missing/);
});
it("rejects a null SQL diagnostic payload at execution", () => {
  const authored = { format: "rerail.policy/v1", id: "null-message", revision: 1, mode: "log", clauses: [{
    id: "diagnostic", relational: {
      inputs: { files: { evidence: { as: "f", query: { ast: { language: "python", rule: { kind: "module" } } }, project: { path: "file.path" } } } },
      query: "SELECT __path path,__start start_byte,__end end_byte,NULL detail FROM files", message: "detail",
    }, violation: { code: "OWNER", message: "Fallback", anchor: "witness.span" },
  }] };
  const rule = loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(codeRuleSchema.parse(authored)), "json", capabilities);
  const result = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0,
    files: [{ path: "owner.py", language: "python", source: "pass\n" }] }), "report"));
  expect(result.violations).toEqual([]);
  expect(result.failures).toEqual([expect.objectContaining({ message: "relational message must be text" })]);
});

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
