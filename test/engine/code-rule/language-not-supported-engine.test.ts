import { describe, expect, it } from "vitest";
import { assertCodeRuleCompiles, CodeRuleCompileError, storableCodeRule } from "../../../src/engine/code-rule/code-rule-compile.js";
import { LANGUAGE_NOT_SUPPORTED, codeRuleLanguageNotSupported } from "../../../src/engine/code-rule/code-rule-language-support.js";
import { CODE_RULE_AST_LANGUAGES, CODE_RULE_PLANNED_LANGUAGES, codeRuleLanguageCapability } from "../../../src/engine/code-rule/code-rule-languages.js";
import { codeRuleSchema } from "../../../src/engine/code-rule/code-rule-schema.js";

// Nothing is mocked: the rule compiles through the native binding the host loads, so the code and reason checked are the
// engine's own. The language comes from the product registry at run time, so a language a later change supports leaves
// this case on one still planned.

const planned: readonly string[] = CODE_RULE_PLANNED_LANGUAGES;

describe("the engine's language-not-supported refusal, through the real native binding", () => {
  it("refuses a rule of a planned language with its code and reason, and the catalog stores it inactive", ({ skip }) => {
    const language = planned[0];
    if (language === undefined) return skip("the language registry plans no language");
    const rule = codeRuleSchema.parse({
      format: "rerail.policy/v1", id: `planned:${language}`, revision: 1, mode: "steer",
      scope: { include: ["**/*"], exclude: [] },
      clauses: [{
        id: "any",
        violation: { code: "ANY", anchor: "witness.span", message: "an identifier" },
        forbid: { ast: { language, rule: { kind: "identifier" } } },
      }],
    });
    const reason = `/clauses/0/forbid/ast/language: \`${language}\` is not supported yet`;
    let refused: unknown;
    try {
      assertCodeRuleCompiles(rule);
    } catch (error) {
      refused = error;
    }
    expect(refused).toBeInstanceOf(CodeRuleCompileError);
    expect(refused).toMatchObject({ reason: `${LANGUAGE_NOT_SUPPORTED}: ${reason}` });
    expect(storableCodeRule(rule)).toEqual({ rule, inactive: { code: LANGUAGE_NOT_SUPPORTED, reason } });
    expect(codeRuleLanguageNotSupported(rule)).toEqual({ code: LANGUAGE_NOT_SUPPORTED, reason });
  });

  // A supported language may still have a capability the engine does not read yet (Java's scope, when this was
  // written): its structural rules run, and a rule reading that capability waits like a planned language's.
  it("compiles a structural rule of a language whose scope is not read yet, and stores one that resolves names inactive", ({ skip }) => {
    const language = CODE_RULE_AST_LANGUAGES.find(candidate => codeRuleLanguageCapability(candidate, "scope") === "not-supported-yet");
    if (language === undefined) return skip("every supported language reads scope");
    const rule = (ast: Record<string, unknown>) => codeRuleSchema.parse({
      format: "rerail.policy/v1", id: `unread:${language}`, revision: 1, mode: "steer",
      scope: { include: ["**/*"], exclude: [] },
      clauses: [{
        id: "any",
        violation: { code: "ANY", anchor: "witness.span", message: "an identifier" },
        forbid: { ast: { language, rule: ast } },
      }],
    });
    const structural = rule({ kind: "identifier" });
    expect(() => assertCodeRuleCompiles(structural)).not.toThrow();
    expect(storableCodeRule(structural)).toEqual({ rule: structural });
    expect(codeRuleLanguageNotSupported(structural)).toBeUndefined();

    const semantic = rule({ kind: "identifier", resolves: { unresolved: true } });
    const reason = `/clauses/0/forbid/ast/rule/resolves: \`resolves\` is not supported yet for ${language} policies`;
    expect(() => assertCodeRuleCompiles(semantic)).toThrow(`${LANGUAGE_NOT_SUPPORTED}: ${reason}`);
    expect(storableCodeRule(semantic)).toEqual({ rule: semantic, inactive: { code: LANGUAGE_NOT_SUPPORTED, reason } });
    expect(codeRuleLanguageNotSupported(semantic)).toEqual({ code: LANGUAGE_NOT_SUPPORTED, reason });
  });

  // A transform step reading a value needs the value capability, as `value` and `compare` do; one reading text runs.
  it("stores a rule whose transform reads a value inactive where values are not read yet, and the host predicts it", ({ skip }) => {
    const language = CODE_RULE_AST_LANGUAGES.find(candidate => codeRuleLanguageCapability(candidate, "value") === "not-supported-yet");
    if (language === undefined) return skip("every supported language reads values");
    const rule = (transform: Record<string, unknown>) => codeRuleSchema.parse({
      format: "rerail.policy/v1", id: `transform:${language}`, revision: 1, mode: "steer",
      scope: { include: ["**/*"], exclude: [] },
      clauses: [{
        id: "any",
        violation: { code: "ANY", anchor: "witness.span", message: "$T" },
        forbid: { ast: { language, rule: { pattern: "f($A)" }, transform } },
      }],
    });
    const text = rule({ T: { substring: { source: "$A", startChar: 1 } } });
    expect(() => assertCodeRuleCompiles(text)).not.toThrow();
    expect(codeRuleLanguageNotSupported(text)).toBeUndefined();

    const at = "/clauses/0/forbid/ast/transform";
    for (const [transform, reason] of [
      [{ T: { toPrecision: { source: "$A", digits: 3, value: true } } }, `${at}/T/toPrecision: \`toPrecision\` is not supported yet for ${language} policies`],
      [{ U: { convert: { source: "$A", value: true, toCase: "upperCase" } }, T: { substring: { source: "$A", value: true } } }, `${at}/T/substring/value: \`value\` is not supported yet for ${language} policies`],
    ] as const) {
      const valued = rule(transform);
      expect(() => assertCodeRuleCompiles(valued)).toThrow(`${LANGUAGE_NOT_SUPPORTED}: ${reason}`);
      expect(storableCodeRule(valued)).toEqual({ rule: valued, inactive: { code: LANGUAGE_NOT_SUPPORTED, reason } });
      expect(codeRuleLanguageNotSupported(valued)).toEqual({ code: LANGUAGE_NOT_SUPPORTED, reason });
    }
  });
});
