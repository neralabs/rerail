import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { escapeHatchSignatureFailure } from "../../../conformance/data-layer/signed-allowlist.js";
import { codeRuleSchema } from "../../../src/engine/index.js";

const policy = () => codeRuleSchema.parse(JSON.parse(readFileSync(new URL(
  "../../../packs/data-layer/candidates/rules/data-layer-sql-grammar-escape-hatches.json", import.meta.url,
), "utf8")));

it("verifies the original signed allowlist before native reporter evaluation", () => {
  expect(escapeHatchSignatureFailure(policy())).toBeUndefined();
});

it("rejects unsigned allowlist edits and invalid signatures with the original reporter diagnostic", () => {
  const changed = policy();
  changed.clauses.find(clause => clause.define !== undefined)!.define!.config = { ALLOWLISTED_CLASSES: ["NewException"] };
  for (const result of [escapeHatchSignatureFailure(changed), escapeHatchSignatureFailure(policy(), "NEEDS_SIGNING"),
    escapeHatchSignatureFailure(policy(), "!")]) {
    expect(result).toMatchObject({ violations: [{ code: "invalid_allowlist_signature",
      path: "scripts/lint_sql_grammar_escape_hatches.py", span: { startLine: 0 } }], failures: [] });
  }
});
