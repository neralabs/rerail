import { describe, expect, it } from "vitest";
import { assertCodeRuleCompiles, CodeRuleCompileError, storableCodeRule } from "../../../src/engine/code-rule/code-rule-compile.js";
import { LANGUAGE_NOT_SUPPORTED, codeRuleLanguageNotSupported } from "../../../src/engine/code-rule/code-rule-language-support.js";
import { CODE_RULE_PLANNED_LANGUAGES } from "../../../src/engine/code-rule/code-rule-languages.js";
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
});
