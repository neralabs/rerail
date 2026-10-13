import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { CODE_RULE_AST_LANGUAGES } from "../../../src/engine/code-rule/code-rule-languages.js";
import { codeRuleMessageVariables, codeRuleSchema } from "../../../src/engine/code-rule/code-rule-schema.js";
import { validateCodeRule } from "../../../src/engine/rules.js";

const astRule = (
  language: "blade" | "javascript" | "php" | "python" | "typescript",
  rule: object,
  { languageOptions, suppress }: { languageOptions?: object; suppress?: object } = {},
) => ({
  format: "rerail.policy/v1",
  id: `test:${language}`,
  revision: 1,
  mode: "steer",
  clauses: [{
    id: "check",
    violation: { code: "TEST", anchor: "witness.span", message: "Test", ...(suppress === undefined ? {} : { suppress }) },
    forbid: { ast: { language, rule, ...(languageOptions === undefined ? {} : { languageOptions }) } },
  }],
});

describe("codeRuleMessageVariables", () => {
  it("recognizes engine-supported braced capture references", () => {
    expect(codeRuleMessageVariables("${NAME} $$$ {IGNORED} $$$${MULTI} \\${ESCAPED}")).toEqual(["NAME", "MULTI", "ESCAPED"]);
  });
});

describe("code rule language mirror", () => {
  it("matches the executable DSL capabilities language keys", () => {
    const capabilities = JSON.parse(readFileSync(new URL("../../../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8")) as {
      ast: { languages: Record<string, string> };
    };
    expect(Object.keys(capabilities.ast.languages).sort()).toEqual([...CODE_RULE_AST_LANGUAGES].sort());
  });
});

describe("code rule schema language families", () => {
  it("accepts structural JavaScript and PHP rules", () => {
    expect(codeRuleSchema.safeParse(astRule("javascript", { kind: "call_expression" })).success).toBe(true);
    expect(codeRuleSchema.safeParse(astRule("php", { kind: "function_call_expression" })).success).toBe(true);
  });

  it("accepts PHP scope, value and argument predicates", () => {
    const accepted = [
      { kind: "function_call_expression", has: { field: "function", resolves: { import: ["exec", "\\App\\Http\\Client"], prefix: true } } },
      { kind: "variable_name", resolves: { global: ["_GET", "_POST"] } },
      { kind: "variable_name", resolves: { global: true } },
      { kind: "variable_name", resolves: { unresolved: true } },
      { kind: "variable_name", resolves: { binding: { form: ["assignment", "static"], scope: "closure", used: true, value: { kind: "string" } } } },
      { kind: "variable_name", binds: { form: "parameter", scope: ["global", "function"], referenced: false, bindings: { max: 1 } } },
      { kind: "string", value: { type: ["string", "int"], truthy: true } },
      { kind: "function_call_expression", arguments: { signature: ["command", "output"], bind: { CMD: "command" }, mode: "language" } },
      { kind: "name", resolves: { imported: ["Illuminate\\Support"], prefix: true } },
      { kind: "variable_name", resolves: { binding: { only: true, stopAtGlobal: true, destructuredValue: false, latest: true, outer: false, occurrence: { kind: "variable_name" } } } },
      { kind: "variable_name", binds: { latest: false, outer: true, occurrence: { kind: "variable_name" }, redefines: { used: false, sameScope: true, every: true } } },
      { kind: "function_call_expression", arguments: { signature: ["command", "output"], bind: { CMD: "command" }, mode: "syntactic" } },
      ...["key", "member", "name", "reference", "reference-no-static-keys", "structure", "structure-hash"].map(text => ({ pattern: "$A == $B", compare: { left: "A", op: "eq", right: "B", text } })),
      { kind: "encapsed_string", parse: { grammar: "printfFormat", has: { kind: "placeholder", has: { kind: "argnum" } } } },
      { pattern: "sprintf($F, $$$A)", formatArgs: { style: "printf", string: "F", args: "A", unused: { kind: "argument" }, missing: { kind: "star" } } },
      { pattern: "sscanf($S, $F, $$$A)", formatArgs: { style: "scanf", string: "F", args: "A", missing: { kind: "placeholder", has: { kind: "set" } } } },
    ];
    for (const rule of accepted) expect(codeRuleSchema.safeParse(astRule("php", rule)).error?.issues ?? []).toEqual([]);
  });

  it("accepts PHP calls rules", () => {
    const sleeps = { stopBy: "end", kind: "function_call_expression", has: { field: "function", resolves: { import: ["sleep"] } } };
    const accepted = [
      { kind: "member_call_expression", calls: { has: sleeps } },
      { kind: "scoped_call_expression", calls: { through: "any", chain: "CHAIN", has: sleeps } },
      { kind: "method_declaration", not: { has: { stopBy: "end", kind: "object_creation_expression", calls: { has: sleeps } } } },
    ];
    for (const rule of accepted) expect(codeRuleSchema.safeParse(astRule("php", rule)).error?.issues ?? []).toEqual([]);
    const nested = { kind: "function_call_expression", calls: { has: { kind: "function_call_expression", calls: { kind: "name" } } } };
    expect(codeRuleSchema.safeParse(astRule("php", nested)).error?.message).toContain("A calls rule cannot contain calls.");
  });

  it.each([
    [{ kind: "comment", parse: { grammar: "phpDoc", kind: "placeholder" } }, "parse.kind", expect.stringMatching(/^placeholder is not a phpDoc kind; expected one of php_doc, php_doc_text, php_doc_tag, .*, name, variable, description\.$/)],
    [{ kind: "string", parse: { grammar: "printfFormat", kind: "mapping_key" } }, "parse.kind", "mapping_key is not a printfFormat kind; expected one of format, literal, placeholder, argnum, flags, width, precision, star, length, conversion, error."],
  ])("refuses the PHP rule %j at %s", (rule, at, message) => {
    const issues = codeRuleSchema.safeParse(astRule("php", rule)).error?.issues ?? [];
    expect(issues.map(issue => ({ path: issue.path.slice(4).join("."), message: issue.message }))).toContainEqual({ path: `rule.${at}`, message });
  });

  it("leaves what a rule's language lacks to the engine", () => {
    const reason = (rule: object) => {
      const validation = validateCodeRule(rule);
      return validation.ok ? undefined : validation.reason;
    };
    expect(codeRuleSchema.safeParse(astRule("javascript", { kind: "identifier", binds: { form: "static", scope: "closure" } })).success).toBe(true);
    expect(reason(astRule("javascript", { kind: "identifier", binds: { form: "static", scope: "closure" } }))).toMatch(/^\/clauses\/0\/forbid\/ast\/rule\/binds\/form: `"static"` is not a binding form; expected one of assignment, .*, accessor$/);
    expect(reason(astRule("php", { kind: "function_call_expression" }, { languageOptions: { ecmaVersion: 2020 } }))).toBe("/clauses/0/forbid/ast/languageOptions: `languageOptions` is not available for php policies");
    expect(reason(astRule("php", { kind: "function_call_expression" }, { suppress: { eslint: { rules: ["test"] } } }))).toBe("/clauses/0/violation/suppress/eslint: eslint-disable comments are ESLint's; a php query has none");
    expect(reason(astRule("javascript", { kind: "call_expression" }, { suppress: { phpcs: { sniffs: ["Squiz.PHP.DiscouragedFunctions"] } } }))).toBe("/clauses/0/violation/suppress/phpcs: phpcs: annotations are PHP_CodeSniffer's; a typescript query has none");
    expect(reason(astRule("python", { kind: "string", parse: { grammar: "printfFormat", kind: "placeholder" } }))).toBe("/clauses/0/forbid/ast/rule/parse/grammar: `printfFormat` parses PHP strings; Python's printf formats are `percentFormat`");
    expect(reason(astRule("typescript", { kind: "comment", parse: { grammar: "printfFormat", kind: "placeholder" } }))).toBe("/clauses/0/forbid/ast/rule/parse/grammar: `printfFormat` parses PHP strings");
    expect(reason(astRule("python", { pattern: "f($S, $$$A)", formatArgs: { style: "printf", string: "S", args: "A", unused: { kind: "identifier" } } }))).toBe("/clauses/0/forbid/ast/rule/formatArgs/style: `printf` reads PHP's printf-family calls; Python's printf formats are style `percent`");
    const mixedParam = { kind: "comment", parse: { grammar: "phpDoc", kind: "param_tag_value", has: { kind: "identifier_type", value: { eq: "mixed" }, capture: "T" } } };
    expect(reason(astRule("typescript", mixedParam))).toBe("/clauses/0/forbid/ast/rule/parse/grammar: `phpDoc` parses PHP doc comments");
    expect(reason(astRule("python", mixedParam))).toBe("/clauses/0/forbid/ast/rule/parse/grammar: `phpDoc` parses PHP doc comments");
    expect(reason(astRule("blade", { kind: "echo_statement" }, { languageOptions: { ecmaVersion: 2020 } }))).toBe("/clauses/0/forbid/ast/languageOptions: `languageOptions` is not available for blade policies");
    expect(reason(astRule("blade", { kind: "echo_statement" }, { suppress: { phpstan: { identifiers: ["variable.undefined"] } } }))).toBe("/clauses/0/violation/suppress/phpstan: @phpstan-ignore comments are PHPStan's; a blade query has none");
  });

  it("accepts PHP's own linters' suppressions, and refuses malformed names", () => {
    const withPhpstan = astRule("php", { kind: "function_call_expression" }, { suppress: { phpstan: { identifiers: ["variable.undefined"] } } });
    expect(codeRuleSchema.safeParse(withPhpstan).success).toBe(true);
    const withAll = astRule("php", { kind: "function_call_expression" }, {
      suppress: {
        phpstan: { identifiers: ["variable.undefined"] },
        psalm: { issues: ["UndefinedVariable"], suppressAll: true },
        phpcs: { sniffs: ["Squiz.PHP.DiscouragedFunctions.Discouraged"], version: 3 },
      },
    });
    expect(codeRuleSchema.safeParse(withAll).success).toBe(true);
    const withAllIssue = astRule("php", { kind: "function_call_expression" }, { suppress: { psalm: { issues: ["all"] } } });
    expect(codeRuleSchema.safeParse(withAllIssue).error?.message).toContain("Must be a Psalm issue name");
    const withCategory = astRule("php", { kind: "function_call_expression" }, { suppress: { phpcs: { sniffs: ["Squiz.PHP"] } } });
    expect(codeRuleSchema.safeParse(withCategory).error?.message).toContain("Must be a PHP_CodeSniffer sniff or message code");
  });

  it("reads Bandit's nosec on Python queries only", () => {
    const withBoth = astRule("python", { kind: "assert_statement" }, { suppress: { noqa: { codes: ["S101"] }, nosec: { codes: ["B101"] } } });
    expect(codeRuleSchema.safeParse(withBoth).success).toBe(true);
    const lowercase = astRule("python", { kind: "assert_statement" }, { suppress: { nosec: { codes: ["b101"] } } });
    expect(codeRuleSchema.safeParse(lowercase).error?.message).toContain("Must be a Bandit test id");
    const onPhp = astRule("php", { kind: "function_call_expression" }, { suppress: { nosec: { codes: ["B101"] } } });
    expect(validateCodeRule(onPhp)).toMatchObject({ ok: false, code: "invalid-rule", reason: expect.stringContaining("nosec comments are Bandit's; a php query has none") });
  });

  it("reads PHP doc comments with phpDoc", () => {
    const mixedParam = { kind: "comment", parse: { grammar: "phpDoc", kind: "param_tag_value", has: { kind: "identifier_type", value: { eq: "mixed" }, capture: "T" } } };
    expect(codeRuleSchema.safeParse(astRule("php", mixedParam)).error?.issues ?? []).toEqual([]);
  });

  it("accepts Blade predicates over the PHP its template embeds", () => {
    const sleeps = { stopBy: "end", kind: "function_call_expression", has: { field: "function", resolves: { import: ["sleep"] } } };
    const accepted = [
      { kind: "variable_name", resolves: { unresolved: true } },
      { kind: "variable_name", resolves: { global: ["_GET"] } },
      { kind: "function_call_expression", has: { field: "function", resolves: { import: ["route"], prefix: true } } },
      { kind: "variable_name", resolves: { binding: { form: ["assignment", "for"], scope: "closure", value: { kind: "integer" } } } },
      { kind: "variable_name", resolves: { binding: { only: true, stopAtGlobal: true, destructuredValue: false, latest: true, outer: false, occurrence: { kind: "variable_name" } } } },
      { kind: "variable_name", binds: { form: "assignment", used: false, latest: true, redefines: { used: false, sameScope: true, every: true } } },
      { kind: "integer", value: { type: ["int", "string"], truthy: true } },
      { kind: "function_call_expression", arguments: { signature: ["name", "parameters"], bind: { N: "name" }, mode: "language" } },
      { kind: "function_call_expression", arguments: { signature: ["name", "parameters"], bind: { N: "name" }, mode: "syntactic" } },
      ...["key", "member", "name", "reference", "reference-no-static-keys", "structure", "structure-hash"].map(text => ({ pattern: "$A == $B", compare: { left: "A", op: "eq", right: "B", text } })),
      { pattern: "sprintf($F, $$$A)", formatArgs: { style: "printf", string: "F", args: "A", unused: { kind: "argument" }, missing: { kind: "placeholder" } } },
      { pattern: "sscanf($S, $F, $$$A)", formatArgs: { style: "scanf", string: "F", args: "A", missing: { kind: "placeholder" } } },
      { kind: "encapsed_string", parse: { grammar: "printfFormat", has: { kind: "placeholder" } } },
      { parse: { grammar: "phpDoc", has: { stopBy: "end", kind: "param_tag_value" } } },
      { kind: "function_call_expression", calls: { through: "any", has: sleeps } },
    ];
    for (const rule of accepted) expect(codeRuleSchema.safeParse(astRule("blade", rule)).error?.issues ?? []).toEqual([]);
  });
});
