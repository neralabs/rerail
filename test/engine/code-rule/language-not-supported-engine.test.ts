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

  // A rule may read several capabilities the engine does not read yet; the reason names the first the engine reaches, so
  // the host walks in the engine's order: shared utilities first (by language, then id), then each clause's queries,
  // each query's utilities, rule and constraints, every object's keys sorted.
  it("names the first unread capability the engine reaches, however the document orders them", ({ skip }) => {
    const language = CODE_RULE_AST_LANGUAGES.find(candidate =>
      codeRuleLanguageCapability(candidate, "scope") === "not-supported-yet" && codeRuleLanguageCapability(candidate, "value") === "not-supported-yet");
    if (language === undefined) return skip("every supported language reads scope and values");
    const violation = { code: "ANY", anchor: "witness.span", message: "an identifier" };
    const document = (clauses: unknown[]) => codeRuleSchema.parse({
      format: "rerail.policy/v1", id: `order:${language}`, revision: 1, mode: "steer", scope: { include: ["**/*"], exclude: [] }, clauses,
    });
    const ast = (query: Record<string, unknown>) => ({ ast: { language, ...query } });
    const relation = (as: string, rule: unknown) => ({ evidence: { as, paths: { include: ["**/*"] }, query: ast({ rule }), project: { name: "file.name" } } });
    const resolves = { kind: "identifier", resolves: { unresolved: true } };
    const binds = { kind: "identifier", binds: { used: false } };
    const notYet = (at: string, key: string) => `${at}: \`${key}\` is not supported yet for ${language} policies`;
    const cases: [string, unknown[], string | undefined][] = [
      ["a rule's keys in sorted order", [{ id: "c", violation, forbid: { ast: { language, rule: { kind: "identifier", value: { eq: 1 }, binds: { used: false } } } } }],
        notYet("/clauses/0/forbid/ast/rule/binds", "binds")],
      ["a query's utilities before its rule", [{ id: "c", violation, forbid: ast({ rule: { all: [resolves, { matches: "u" }] }, utils: { u: binds } }) }],
        notYet("/clauses/0/forbid/ast/utils/u/binds", "binds")],
      ["utilities and constraints by name", [{ id: "c", violation, forbid: ast({ rule: { pattern: "f($A, $B)" }, constraints: { B: resolves, A: binds } }) }],
        notYet("/clauses/0/forbid/ast/constraints/A/binds", "binds")],
      ["shared utilities before any query", [
        { id: "c", violation, forbid: ast({ rule: resolves }) },
        { id: "d", define: { language, utils: { z: binds, a: { kind: "identifier", value: { eq: 1 } } } } },
      ], notYet("/clauses/1/define/utils/a/value", "value")],
      ["a subject before its evidence", [{ id: "c", violation, require_each: { evidence: relation("evidence", binds), subject: relation("subject", resolves) } }],
        notYet("/clauses/0/require_each/subject/evidence/query/ast/rule/resolves", "resolves")],
      ["a utility named as a predicate is no predicate", [{ id: "c", violation, forbid: ast({ rule: { matches: "value" }, utils: { value: { kind: "identifier" } } }) }],
        undefined],
    ];
    for (const [name, clauses, reason] of cases) {
      const rule = document(clauses);
      const engine = (() => {
        try {
          assertCodeRuleCompiles(rule);
          return undefined;
        } catch (error) {
          return error instanceof CodeRuleCompileError ? error.reason : String(error);
        }
      })();
      expect(engine, name).toBe(reason === undefined ? undefined : `${LANGUAGE_NOT_SUPPORTED}: ${reason}`);
      expect(codeRuleLanguageNotSupported(rule), name).toEqual(reason === undefined ? undefined : { code: LANGUAGE_NOT_SUPPORTED, reason });
    }
  });
});
