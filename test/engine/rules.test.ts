import { describe, expect, it, vi } from "vitest";
import { validateCodeRule } from "../../src/engine/rules.js";
import { EXEMPLAR } from "../support/exemplar-registry.js";

// The exemplar is a language only tests plan: the registry module lists it for this file, and the engine refuses it as a
// planned language (test/support/exemplar-registry.ts, exemplar-engine.ts).
vi.mock("../../src/engine/code-rule/code-rule-language-registry.generated.js", async () =>
  (await import("../support/exemplar-registry.js")).exemplarRegistryModule());
vi.mock("../../src/engine/native/native.js", async importOriginal =>
  (await import("../support/exemplar-engine.js")).nativeRefusingExemplar(await importOriginal()));

/** One forbid clause over an ast query of `language`, reporting every identifier. */
const astRule = (language: string, message = "an identifier") => ({
  format: "rerail.policy/v1",
  id: `test:${language}`,
  revision: 1,
  mode: "steer",
  clauses: [{
    id: "any",
    violation: { code: "ANY", anchor: "witness.span", message },
    forbid: { ast: { language, rule: { kind: "identifier" } } },
  }],
});

describe("validateCodeRule", () => {
  it("accepts a rule of a supported language, active", () => {
    const rule = astRule("python");
    expect(validateCodeRule(rule)).toEqual({ ok: true, rule });
  });

  it("accepts a rule of a planned language, inactive with the engine's reason", () => {
    const rule = astRule(EXEMPLAR);
    expect(validateCodeRule(rule)).toEqual({
      ok: true,
      rule,
      inactive: { code: "language-not-supported", reason: `/clauses/0/forbid/ast/language: \`${EXEMPLAR}\` is not supported yet` },
    });
  });

  it("refuses a language the registry does not list, with the schema's issues", () => {
    const validation = validateCodeRule(astRule("ruby"));
    expect(validation).toMatchObject({ ok: false, code: "invalid-rule" });
    if (validation.ok) return;
    expect(validation.issues?.map(issue => issue.path.join("/"))).toEqual(["clauses/0/forbid/ast/language"]);
    expect(validation.reason).toContain("clauses[0].forbid.ast.language");
  });

  it("refuses a schema-valid rule the engine refuses, with the engine's reason", () => {
    expect(validateCodeRule(astRule("typescript", "bad $TYPO"))).toEqual({
      ok: false,
      code: "invalid-rule",
      reason: "/clauses/0/violation/message: `$TYPO` names no capture; the query binds no captures; write `\\$TYPO` for literal text",
    });
  });

  it("refuses a document that is not JSON, without throwing", () => {
    const rule: Record<string, unknown> = astRule("python");
    rule.self = rule;
    expect(validateCodeRule(rule)).toEqual({ ok: false, code: "invalid-rule", reason: "the document is not JSON representable" });
  });
});
