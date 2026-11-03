import { expect, it } from "vitest";
import { codeRuleSchema } from "../../src/engine/code-rule/code-rule-schema.js";

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
