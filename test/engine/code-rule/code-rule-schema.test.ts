import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { CODE_RULE_AST_LANGUAGES } from "../../../src/engine/code-rule/code-rule-languages.js";
import { codeRuleMessageVariables, codeRuleSchema } from "../../../src/engine/code-rule/code-rule-schema.js";
import { validateCodeRule } from "../../../src/engine/rules.js";

const astRule = (
  language: "blade" | "javascript" | "php" | "python" | "typescript" | "vue",
  rule: object,
  { languageOptions, suppress, embedded, utils }: { languageOptions?: object; suppress?: object; embedded?: string; utils?: object } = {},
) => ({
  format: "rerail.policy/v1",
  id: `test:${language}`,
  revision: 1,
  mode: "steer",
  clauses: [{
    id: "check",
    violation: { code: "TEST", anchor: "witness.span", message: "Test", ...(suppress === undefined ? {} : { suppress }) },
    forbid: { ast: { language, ...(embedded === undefined ? {} : { embedded }), rule, ...(utils === undefined ? {} : { utils }), ...(languageOptions === undefined ? {} : { languageOptions }) } },
  }],
});

/** A document's schema issues, with paths relative to its query's `ast`. */
const astIssues = (document: object) =>
  (codeRuleSchema.safeParse(document).error?.issues ?? []).map(issue => ({ path: issue.path.slice(4).join("."), message: issue.message }));

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
    expect(reason(astRule("blade", { kind: "echo" }, { languageOptions: { ecmaVersion: 2020 } }))).toBe("/clauses/0/forbid/ast/languageOptions: `languageOptions` is not available for blade policies");
    expect(reason(astRule("blade", { kind: "echo" }, { suppress: { phpstan: { identifiers: ["variable.undefined"] } } }))).toBe("/clauses/0/violation/suppress/phpstan: @phpstan-ignore comments are PHPStan's; a blade query has none");
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

  it("reads oxlint's disable directives on TypeScript and JavaScript queries only", () => {
    const hook = { kind: "call_expression" };
    const both = { eslint: { rules: ["react-hooks/rules-of-hooks"] }, oxlint: { rules: ["rules-of-hooks"] } };
    expect(codeRuleSchema.safeParse(astRule("typescript", hook, { suppress: both })).success).toBe(true);
    expect(codeRuleSchema.safeParse(astRule("javascript", hook, { suppress: { oxlint: { rules: ["rules-of-hooks"] } } })).success).toBe(true);
    const empty = astRule("typescript", hook, { suppress: { oxlint: { rules: [] } } });
    expect(codeRuleSchema.safeParse(empty).error?.message).toContain("Too small");
    const prefixed = astRule("typescript", hook, { suppress: { oxlint: { rules: ["react-hooks/rules-of-hooks"] } } });
    expect(codeRuleSchema.safeParse(prefixed).error?.message).toContain("Must be an oxlint rule name");
    const onPython = astRule("python", { kind: "assert_statement" }, { suppress: { oxlint: { rules: ["no-console"] } } });
    expect(validateCodeRule(onPython)).toMatchObject({ ok: false, code: "invalid-rule", reason: expect.stringContaining("oxlint-disable comments are Oxlint's; a python query has none") });
    const onPhp = astRule("php", { kind: "function_call_expression" }, { suppress: { oxlint: { rules: ["no-console"] } } });
    expect(validateCodeRule(onPhp)).toMatchObject({ ok: false, code: "invalid-rule", reason: expect.stringContaining("oxlint-disable comments are Oxlint's; a php query has none") });
  });

  it("reads Biome's biome-ignore comments on TypeScript and JavaScript queries only, naming lint rules of its JavaScript analyzer", () => {
    const hook = { kind: "call_expression" };
    const all = { eslint: { rules: ["react-hooks/rules-of-hooks"] }, oxlint: { rules: ["rules-of-hooks"] }, biome: { rules: ["lint/correctness/useHookAtTopLevel"] } };
    expect(validateCodeRule(astRule("typescript", hook, { suppress: all }))).toMatchObject({ ok: true });
    expect(validateCodeRule(astRule("javascript", hook, { suppress: { biome: { rules: ["lint/correctness/useHookAtTopLevel"] } } }))).toMatchObject({ ok: true });
    const empty = astRule("typescript", hook, { suppress: { biome: { rules: [] } } });
    expect(codeRuleSchema.safeParse(empty).error?.message).toContain("Too small");
    const bare = astRule("typescript", hook, { suppress: { biome: { rules: ["useHookAtTopLevel"] } } });
    expect(codeRuleSchema.safeParse(bare).error?.message).toContain("Must be a Biome lint rule name");
    const group = astRule("typescript", hook, { suppress: { biome: { rules: ["lint/correctness"] } } });
    expect(codeRuleSchema.safeParse(group).error?.message).toContain("Must be a Biome lint rule name");
    const css = astRule("typescript", hook, { suppress: { biome: { rules: ["lint/a11y/useGenericFontNames"] } } });
    expect(validateCodeRule(css)).toMatchObject({ ok: false, code: "invalid-rule", reason: expect.stringContaining("must be a lint rule of Biome 2.5.15's JavaScript analyzer") });
    const onPython = astRule("python", { kind: "assert_statement" }, { suppress: { biome: { rules: ["lint/suspicious/noConsole"] } } });
    expect(validateCodeRule(onPython)).toMatchObject({ ok: false, code: "invalid-rule", reason: expect.stringContaining("biome-ignore comments are Biome's; a python query has none") });
  });

  it("reads PHP doc comments with phpDoc", () => {
    const mixedParam = { kind: "comment", parse: { grammar: "phpDoc", kind: "param_tag_value", has: { kind: "identifier_type", value: { eq: "mixed" }, capture: "T" } } };
    expect(codeRuleSchema.safeParse(astRule("php", mixedParam)).error?.issues ?? []).toEqual([]);
  });

  it("reads import targets with a style and eslint-plugin-n's facts", () => {
    const source = { field: "source", any: [{ kind: "import_statement" }, { kind: "export_statement" }] };
    const accepted = [
      { kind: "string", inside: source, importTarget: { via: "import", builtin: false, failed: true } },
      { kind: "string", importTarget: { via: "require", moduleType: "npm", declared: false } },
      { kind: "string", importTarget: { via: "import", moduleType: ["relative", "absolute"], resolved: true, extensionSpelled: false, published: false } },
      { kind: "string", not: { importTarget: { via: "require", devDependency: true, aliased: false } } },
    ];
    for (const rule of accepted) expect(astIssues(astRule("typescript", rule))).toEqual([]);
    expect(astIssues(astRule("javascript", { kind: "string", has: { kind: "string_fragment", importTarget: { via: "import" } } }))).toEqual([]);
    const refused = (importTarget: object) => astIssues(astRule("typescript", { kind: "string", importTarget })).map(issue => issue.path);
    expect(refused({ failed: true })).toContain("rule.importTarget.via");
    expect(refused({ via: "export" })).toContain("rule.importTarget.via");
    expect(refused({ via: "import", moduleType: [] })).toContain("rule.importTarget.moduleType");
    expect(refused({ via: "import", moduleType: "builtin" })).toContain("rule.importTarget.moduleType");
    expect(refused({ via: "import", failed: "yes" })).toContain("rule.importTarget.failed");
    expect(refused({ via: "import", missing: true })).toContain("rule.importTarget");
    expect(validateCodeRule(astRule("python", { kind: "string", importTarget: { via: "import", failed: true } }))).toMatchObject({
      ok: false,
      reason: expect.stringContaining("/clauses/0/forbid/ast/rule/importTarget: import targets are eslint-plugin-n's, read in JavaScript and TypeScript files"),
    });
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
    for (const rule of accepted) expect(astIssues(astRule("blade", rule, { embedded: "compiled-view" }))).toEqual([]);
  });
});

describe("code rule schema embedded programs", () => {
  it("runs a query on the program its language embeds through ast.embedded", () => {
    expect(astIssues(astRule("blade", { kind: "function_call_expression" }, { embedded: "compiled-view" }))).toEqual([]);
    expect(astIssues(astRule("vue", { kind: "call_expression" }, { embedded: "script-block" }))).toEqual([]);
  });

  it.each([
    ["blade", "script-block", "blade embeds no program via script-block; expected compiled-view."],
    ["vue", "compiled-view", "vue embeds no program via compiled-view; expected script-block."],
    ["php", "compiled-view", "php embeds no other language; ast.embedded names how a language embeds one (blade: compiled-view; vue: script-block)."],
  ] as const)("refuses ast.embedded on %s via %s", (language, via, message) => {
    expect(astIssues(astRule(language, { kind: "program" }, { embedded: via }))).toContainEqual({ path: "embedded", message });
  });

  it("reaches the embedded program from a host node with embedded, and back with host", () => {
    const echoesGet = { kind: "echo_statement", embedded: { kind: "variable_name", regex: "^\\$_GET$", host: { kind: "echo_statement" } } };
    expect(astIssues(astRule("blade", echoesGet))).toEqual([]);
    expect(astIssues(astRule("vue", { kind: "interpolation", embedded: { kind: "identifier", not: { inside: { kind: "arrow_function" } } } }))).toEqual([]);
    // A host rule reached from the program may reach the program again.
    const nested = { kind: "variable_name", host: { kind: "directive", embedded: { kind: "variable_name" } } };
    expect(astIssues(astRule("blade", nested, { embedded: "compiled-view" }))).toEqual([]);
  });

  it("reads host outside parse only from a node of an embedded program", () => {
    expect(astIssues(astRule("blade", { kind: "variable_name", host: { kind: "echo_statement" } }, { embedded: "compiled-view" }))).toEqual([]);
    expect(astIssues(astRule("vue", { kind: "identifier", any: [{ host: { kind: "interpolation" } }, { inside: { kind: "call_expression", host: { kind: "directive_attribute" } } }] }, { embedded: "script-block" }))).toEqual([]);
    expect(astIssues(astRule("blade", { kind: "echo_statement", host: { kind: "directive" } }))).toContainEqual({
      path: "rule.host",
      message: "host tests the literal a parse holds, or the host node of an embedded program's node; use it inside parse, or in a rule over the embedded program (ast.embedded or embedded).",
    });
    // Inside a parse the innermost context decides: host is the literal the parse holds.
    const docComment = { kind: "comment", parse: { grammar: "phpDoc", has: { kind: "param_tag_value", host: { kind: "comment" } } } };
    expect(astIssues(astRule("blade", docComment, { embedded: "compiled-view" }))).toEqual([]);
  });

  it.each([
    ["a rule over the embedded program", "blade", { kind: "variable_name", embedded: { kind: "variable_name" } }, "compiled-view", "rule.embedded", "embedded reaches the program embedded in a host node; this rule already runs on the embedded program. Reach the host node with host first."],
    ["an embedded sub-rule", "vue", { kind: "element", embedded: { kind: "identifier", has: { kind: "identifier", embedded: { kind: "identifier" } } } }, undefined, "rule.embedded.has.embedded", "embedded reaches the program embedded in a host node; this rule already runs on the embedded program. Reach the host node with host first."],
    ["a language that embeds none", "php", { kind: "program", embedded: { kind: "variable_name" } }, undefined, "rule.embedded", "embedded reaches the program a language embeds; php embeds none (blade: compiled-view; vue: script-block)."],
    ["a parsed tree", "blade", { kind: "comment", parse: { grammar: "phpDoc", has: { kind: "param_tag_value", embedded: { kind: "name" } } } }, "compiled-view", "rule.parse.has.embedded", "embedded is not supported inside parse (phpDoc trees)."],
  ] as const)("refuses embedded in %s", (_name, language, rule, embedded, path, message) => {
    expect(astIssues(astRule(language, rule, embedded === undefined ? {} : { embedded }))).toContainEqual({ path, message });
  });

  it("leaves the document of utilities to the rules that reach them", () => {
    const utils = { "in-echo": { host: { kind: "echo_statement" } }, "echoes-get": { embedded: { kind: "variable_name", regex: "^\\$_GET$" } } };
    expect(astIssues(astRule("blade", { kind: "echo_statement", matches: "echoes-get" }, { utils }))).toEqual([]);
    expect(astIssues(astRule("blade", { kind: "variable_name", matches: "in-echo" }, { embedded: "compiled-view", utils }))).toEqual([]);
    expect(astIssues(astRule("php", { kind: "variable_name", matches: "in-echo" }, { utils: { "in-echo": { host: { kind: "echo_statement" } } } }))).toContainEqual({
      path: "utils.in-echo.host",
      message: "host tests the literal a parse holds; use it inside parse.",
    });
  });
});
