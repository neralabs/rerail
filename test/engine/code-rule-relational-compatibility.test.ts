import { expect, it } from "vitest";
import { codeRuleAstRuleSchema, codeRuleEvidenceQuerySchema, codeRuleQuerySchema, codeRuleSchema } from "../../src/engine/code-rule/code-rule-schema.js";

it("keeps established relational rules available through the public authoring schema", () => {
  const policy = {
    format: "rerail.policy/v1", id: "relational-compatibility", revision: 1, mode: "block",
    clauses: [{ id: "rows", relational: {
      inputs: { files: { evidence: { as: "file", query: { ast: { language: "python", rule: { kind: "module" } } }, project: { path: "file.path" } } } },
      query: "SELECT __path path,__start start_byte,__end end_byte FROM files", key: ["path"],
    }, violation: { code: "rows", message: "Rows", anchor: "witness.span" } }],
  };
  expect(codeRuleSchema.parse(policy).clauses[0]).toHaveProperty("relational");
});

it("retains the source and path gates used by existing ports", () => {
  const rule = { all: [{ filePath: { regex: "^impl/" } }, { layout: { line: "marker", previousLine: "allow", context: { before: 2, regex: "reviewed" } } }] };
  expect(codeRuleAstRuleSchema.parse(rule)).toEqual(rule);
  const query = { regex: { source: "file.content", pattern: "forbidden", content: { include: ["trigger"] },
    include: ["forbidden"], exclude: ["reviewed"], lines: { first: true, terminators: "python" }, paths: { include: ["impl/**"], exclude: [] } } };
  expect(codeRuleQuerySchema.parse(query)).toEqual(query);
});

it("retains capture columns, enumeration, and partitioned helper closures", () => {
  const evidence = { evidence: { as: "edge", query: { ast: { language: "python", syntax: "valid", rule: { pattern: "$NODE" } } },
    project: { path: "file.path", name: "capture.NODE.text", start: "capture.NODE.startByte" },
    each: "NODE", anchor: "file.start", where: { ge: ["edge.start", 0] },
    closure: [{ name: "reachable", seed_field: "name", partition_by: ["path"], contains: { reached: "name" } }],
  } };
  expect(codeRuleEvidenceQuerySchema.parse(evidence)).toEqual(evidence);
});

it("retains the existing report threshold without discarding failures", () => {
  const policy = { format: "rerail.policy/v1", id: "threshold", revision: 1, mode: "block", clauses: [
    { id: "threshold", define: { report_when: { count: { gt: 3 } } } },
    { id: "findings", forbid: { regex: { source: "file.content", pattern: "bad" } }, violation: { code: "bad", message: "Bad", anchor: "witness.span" } },
  ] };
  expect(codeRuleSchema.parse(policy).clauses[0]).toHaveProperty("define.report_when.count.gt", 3);
});
