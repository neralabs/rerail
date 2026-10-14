import { describe, expect, it } from "vitest";
import { assertCodeRuleCompiles, CodeRuleCompileError, storableCodeRule } from "../../../src/engine/code-rule/code-rule-compile.js";
import {
  CODE_RULE_PREDICATE_CAPABILITIES,
  LANGUAGE_NOT_SUPPORTED,
  codeRuleLanguageNotSupported,
} from "../../../src/engine/code-rule/code-rule-language-support.js";
import {
  CODE_RULE_AST_LANGUAGES,
  CODE_RULE_PLANNED_LANGUAGES,
  CODE_RULE_SEMANTIC_CAPABILITIES,
  codeRuleLanguageCapability,
  type CodeRuleSemanticCapability,
} from "../../../src/engine/code-rule/code-rule-languages.js";
import { codeRuleSchema, type CodeRule } from "../../../src/engine/code-rule/code-rule-schema.js";

// Nothing is mocked: each rule compiles through the native binding the host loads, so the code and reason checked are
// the engine's own. The languages and capabilities come from the product registry at run time, so a language a later
// change supports leaves these cases on the ones still planned.

/** One forbid clause over an ast query of `language` whose rule is `rule`. */
function astCodeRule(id: string, language: string, rule: Record<string, unknown>): CodeRule {
  return codeRuleSchema.parse({
    format: "rerail.policy/v1", id, revision: 1, mode: "steer",
    scope: { include: ["**/*"], exclude: [] },
    clauses: [{
      id: "any",
      violation: { code: "ANY", anchor: "witness.span", message: "a match" },
      forbid: { ast: { language, rule } },
    }],
  });
}

/** The engine's compile error for `rule`, or undefined when it compiles. */
function engineRefusal(rule: CodeRule): unknown {
  try {
    assertCodeRuleCompiles(rule);
  } catch (error) {
    return error;
  }
  return undefined;
}

/** A predicate reading each capability, and a rule holding it in a shape the schema accepts. */
const PREDICATES: Readonly<Record<CodeRuleSemanticCapability, [string, Record<string, unknown>]>> = {
  scope: ["resolves", { kind: "identifier", resolves: { global: ["open"] } }],
  value: ["value", { kind: "identifier", value: { truthy: false } }],
  arguments: ["arguments", { kind: "identifier", arguments: { signature: ["a"], absent: ["a"] } }],
  calls: ["calls", { kind: "identifier", calls: { has: { kind: "identifier" } } }],
  formatArgs: ["formatArgs", { pattern: "f($F, $$$A)", formatArgs: { style: "percent", string: "F", args: "A", unused: { kind: "identifier" } } }],
};

const planned: readonly string[] = CODE_RULE_PLANNED_LANGUAGES;
const notSupportedYet = CODE_RULE_AST_LANGUAGES.flatMap(language =>
  CODE_RULE_SEMANTIC_CAPABILITIES.filter(capability => codeRuleLanguageCapability(language, capability) === "not-supported-yet").map(capability => ({ language, capability })),
);

describe("the engine's language-not-supported refusal, through the real native binding", () => {
  it("refuses a rule of a planned language with its code and reason, and the catalog stores it inactive", ({ skip }) => {
    const language = planned[0];
    if (language === undefined) return skip("the language registry plans no language");
    const rule = astCodeRule(`planned:${language}`, language, { kind: "identifier" });
    const reason = `/clauses/0/forbid/ast/language: \`${language}\` is not supported yet`;
    const refused = engineRefusal(rule);
    expect(refused).toBeInstanceOf(CodeRuleCompileError);
    expect(refused).toMatchObject({ reason: `${LANGUAGE_NOT_SUPPORTED}: ${reason}` });
    expect(storableCodeRule(rule)).toEqual({ rule, inactive: { code: LANGUAGE_NOT_SUPPORTED, reason } });
    expect(codeRuleLanguageNotSupported(rule)).toEqual({ code: LANGUAGE_NOT_SUPPORTED, reason });
  });

  it("refuses a rule reading a capability its supported language has not-supported-yet, with its code and reason", ({ skip }) => {
    const found = notSupportedYet[0];
    if (found === undefined) return skip("no supported language has a not-supported-yet capability");
    const { language, capability } = found;
    const [predicate, ruleBody] = PREDICATES[capability];
    expect(CODE_RULE_PREDICATE_CAPABILITIES[predicate]).toBe(capability);
    const rule = astCodeRule(`capability:${language}:${capability}`, language, ruleBody);
    const reason = `/clauses/0/forbid/ast/rule/${predicate}: \`${predicate}\` is not supported yet for ${language} policies`;
    const refused = engineRefusal(rule);
    expect(refused).toBeInstanceOf(CodeRuleCompileError);
    expect(refused).toMatchObject({ reason: `${LANGUAGE_NOT_SUPPORTED}: ${reason}` });
    expect(storableCodeRule(rule)).toEqual({ rule, inactive: { code: LANGUAGE_NOT_SUPPORTED, reason } });
    expect(codeRuleLanguageNotSupported(rule)).toEqual({ code: LANGUAGE_NOT_SUPPORTED, reason });
  });
});
