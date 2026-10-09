import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { CODE_RULE_AST_LANGUAGES } from "../src/code-rule-languages.js";
import { codeRuleMessageVariables, codeRuleSchema } from "../src/code-rule-schema.js";

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
    const capabilities = JSON.parse(readFileSync(new URL("../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8")) as {
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
    [{ resolves: { this: "global" } }, "resolves.this", "this asks whether JavaScript's this is the global object; PHP's $this never is: outside an object context it is undefined, which unresolved: true reports; php has none."],
    [{ resolves: { imported: ["exec"], global: ["_GET"] } }, "resolves", "import and imported resolve class, function and constant names; global and unresolved resolve variables; one resolves asks one of them."],
    [{ resolves: { imported: ["Foo"], seenBefore: true } }, "resolves.seenBefore", "Ruff's seen_module asks whether a module was imported before; a PHP use only aliases a name for the rest of its namespace and loads nothing; php has none."],
    [{ resolves: { beforeLocal: true } }, "resolves.beforeLocal", "a PHP function sees no variable of an enclosing scope, so a read before the function binds the name is undefined, which unresolved: true reports; php has none."],
    [{ resolves: { unresolved: "star" } }, "resolves.unresolved", "php policies ask unresolved: true."],
    [{ resolves: { global: ["_GET", "argv"] } }, "resolves.global.1", "argv is no PHP superglobal; expected one of GLOBALS, _SERVER, _GET, _POST, _FILES, _COOKIE, _SESSION, _REQUEST, _ENV."],
    [{ resolves: { import: ["os.path"] } }, "resolves.import.0", "os.path is a dotted name; PHP names join labels with \\."],
    [{ resolves: { write: "last", binding: {} } }, "resolves.write", "ESLint reports an unused variable once, at its last write; php's usage oracle, Psalm, reports each unused assignment, which binds: { used: false } matches."],
    [{ resolves: { binding: { stopAtGlobal: false } } }, "resolves.binding.stopAtGlobal", "a php read after global $x reads that statement's binding; which top-level binding the global variable then holds depends on when the function runs, so php has no lookup through it."],
    [{ binds: { exported: true } }, "binds.exported", "exported reads Python's re-exports and TypeScript's exports; a PHP file exports no variables (an including file shares its global scope, which no declaration exports); php has none."],
    [{ binds: { builtin: true } }, "binds.builtin", "builtin reads ESLint's configured globals; PHP's superglobals are no file's bindings (an assignment to $_GET writes the superglobal); php has none."],
    [{ binds: { directives: "eslint" } }, "binds.directives", "directives reads ESLint's /* exported */ comments; PHP exports no variables; php has none."],
    [{ binds: { form: "import" } }, "binds.form", "import is not a php binding form; expected one of assignment, augmented, for, except, parameter, global, static."],
    [{ binds: { scope: ["function", "module"] } }, "binds.scope.1", "module is not a php scope kind; expected one of global, function, closure."],
    [{ binds: { redefines: { sameImport: true } } }, "binds.redefines.sameImport", "Qualified import names are Python's; a PHP variable is never bound by an import."],
    [{ value: { type: "str" } }, "value.type.0", "str is not a php type name; expected one of int, float, string, bool, null, array."],
    [{ arguments: { signature: ["a", "/", "b"], bind: { A: "a" } } }, "arguments.signature", "/ and * mark Python's positional-only and keyword-only slots; PHP has no positional-only or keyword-only parameters."],
    [{ arguments: { signature: ["a"], bind: { A: "a" }, mode: "estree" } }, "arguments.mode", "estree reads ESLint's node.arguments of ESTree, JavaScript's syntax tree; php has none (use language or syntactic)."],
    [{ pattern: "$A == $B", compare: { left: "A", op: "eq", right: "B", text: "structure-parts" } }, "compare.text", "structure-parts compares Python's implicitly concatenated string parts; PHP has no implicit string concatenation."],
    [{ kind: "string", parse: { grammar: "percentFormat", kind: "placeholder" } }, "parse.grammar", "percentFormat parses Python literals; PHP's printf formats are printfFormat."],
    [{ kind: "string", parse: { grammar: "braceFormat", kind: "field" } }, "parse.grammar", "braceFormat parses Python literals; PHP has no str.format templates (its printf formats are printfFormat)."],
    [{ kind: "string", parse: { grammar: "annotation", kind: "identifier" } }, "parse.grammar", "annotation parses Python literals; PHP holds no type in a string: its declared types are syntax, and its PHPDoc types are phpDoc."],
    [{ kind: "comment", parse: { grammar: "phpDoc", kind: "placeholder" } }, "parse.kind", expect.stringMatching(/^placeholder is not a phpDoc kind; expected one of php_doc, php_doc_text, php_doc_tag, .*, name, variable, description\.$/)],
    [{ kind: "string", parse: { grammar: "printfFormat", kind: "mapping_key" } }, "parse.kind", "mapping_key is not a printfFormat kind; expected one of format, literal, placeholder, argnum, flags, width, precision, star, length, conversion, error."],
    [{ pattern: "sprintf($F, $$$A)", formatArgs: { style: "percent", string: "F", args: "A", missing: { kind: "placeholder" } } }, "formatArgs.style", "percent reads Python's % formats; PHP's printf-family calls are style printf."],
    [{ pattern: "sprintf($F, $$$A)", formatArgs: { style: "brace", string: "F", args: "A", missing: { kind: "field" } } }, "formatArgs.style", "brace reads Python's str.format calls; PHP has no such templates (its printf-family calls are style printf)."],
    [{ kind: "comment", parse: { grammar: "eslintDirective", kind: "directive" } }, "parse.grammar", "eslintDirective parses TypeScript comments."],
  ])("refuses the PHP rule %j at %s", (rule, at, message) => {
    const issues = codeRuleSchema.safeParse(astRule("php", rule)).error?.issues ?? [];
    expect(issues.map(issue => ({ path: issue.path.slice(4).join("."), message: issue.message }))).toContainEqual({ path: `rule.${at}`, message });
  });

  it("keeps static forms and closure scopes PHP's", () => {
    const issues = codeRuleSchema.safeParse(astRule("javascript", { kind: "identifier", binds: { form: "static", scope: "closure" } })).error?.issues ?? [];
    expect(issues.map(issue => issue.message)).toEqual([
      expect.stringMatching(/^static is not a javascript binding form; expected one of assignment, .*, accessor\.$/),
      "closure is not a javascript scope kind; expected one of global, module, function, class, block, namespace, typeParameters, functionName.",
    ]);
  });

  it("refuses PHP language options and other linters' suppressions", () => {
    const withOptions = astRule("php", { kind: "function_call_expression" }, { languageOptions: { ecmaVersion: 2020 } });
    expect(codeRuleSchema.safeParse(withOptions).error?.message).toContain("languageOptions: `languageOptions` is not available for php policies");

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
    const withEslint = astRule("php", { kind: "function_call_expression" }, { suppress: { eslint: { rules: ["test"] } } });
    expect(codeRuleSchema.safeParse(withEslint).error?.message).toContain("eslint-disable comments are ESLint's; a php query has none.");
    const withAllIssue = astRule("php", { kind: "function_call_expression" }, { suppress: { psalm: { issues: ["all"] } } });
    expect(codeRuleSchema.safeParse(withAllIssue).error?.message).toContain("Must be a Psalm issue name");
    const withCategory = astRule("php", { kind: "function_call_expression" }, { suppress: { phpcs: { sniffs: ["Squiz.PHP"] } } });
    expect(codeRuleSchema.safeParse(withCategory).error?.message).toContain("Must be a PHP_CodeSniffer sniff or message code");
    const onJavascript = astRule("javascript", { kind: "call_expression" }, { suppress: { phpcs: { sniffs: ["Squiz.PHP.DiscouragedFunctions"] } } });
    expect(codeRuleSchema.safeParse(onJavascript).error?.message).toContain("phpcs: annotations are PHP_CodeSniffer's; a javascript query has none.");
  });

  it("keeps PHP's printf formats PHP's", () => {
    const issues = (rule: object, language: "python" | "typescript") =>
      (codeRuleSchema.safeParse(astRule(language, rule)).error?.issues ?? []).map(issue => ({ path: issue.path.slice(4).join("."), message: issue.message }));
    expect(issues({ kind: "string", parse: { grammar: "printfFormat", kind: "placeholder" } }, "python")).toContainEqual({
      path: "rule.parse.grammar",
      message: "printfFormat parses PHP strings; Python's printf formats are percentFormat.",
    });
    expect(issues({ kind: "comment", parse: { grammar: "printfFormat", kind: "placeholder" } }, "typescript")).toContainEqual({
      path: "rule.parse.grammar",
      message: "printfFormat parses PHP strings.",
    });
    expect(issues({ pattern: "f($S, $$$A)", formatArgs: { style: "printf", string: "S", args: "A", unused: { kind: "identifier" } } }, "python")).toContainEqual({
      path: "rule.formatArgs.style",
      message: "printf reads PHP's printf-family calls; Python's printf formats are style percent.",
    });
  });

  it("reads PHP doc comments with phpDoc, and only PHP's", () => {
    const issues = (rule: object, language: "php" | "python" | "typescript") =>
      (codeRuleSchema.safeParse(astRule(language, rule)).error?.issues ?? []).map(issue => ({ path: issue.path.slice(4).join("."), message: issue.message }));
    const mixedParam = { kind: "comment", parse: { grammar: "phpDoc", kind: "param_tag_value", has: { kind: "identifier_type", value: { eq: "mixed" }, capture: "T" } } };
    expect(issues(mixedParam, "php")).toEqual([]);
    expect(issues(mixedParam, "typescript")).toContainEqual({ path: "rule.parse.grammar", message: "phpDoc parses PHP doc comments." });
    expect(issues(mixedParam, "python")).toContainEqual({ path: "rule.parse.grammar", message: "phpDoc parses PHP doc comments." });
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

  it.each([
    [{ resolves: { this: "global" } }, "resolves.this", "this asks whether JavaScript's this is the global object; PHP's $this never is: outside an object context it is undefined, which unresolved: true reports; blade has none."],
    [{ resolves: { unresolved: "star" } }, "resolves.unresolved", "blade policies ask unresolved: true."],
    [{ resolves: { write: "last", binding: {} } }, "resolves.write", "ESLint reports an unused variable once, at its last write; blade reads its embedded PHP, whose usage oracle, Psalm, reports each unused assignment, which binds: { used: false } matches."],
    [{ resolves: { binding: { stopAtGlobal: false } } }, "resolves.binding.stopAtGlobal", "a blade read after global $x reads that statement's binding; which top-level binding the global variable then holds depends on when the function runs, so blade has no lookup through it."],
    [{ binds: { exported: true } }, "binds.exported", "exported reads Python's re-exports and TypeScript's exports; a PHP file exports no variables (an including file shares its global scope, which no declaration exports); blade has none."],
    [{ binds: { builtin: true } }, "binds.builtin", "builtin reads ESLint's configured globals; PHP's superglobals are no file's bindings (an assignment to $_GET writes the superglobal); blade has none."],
    [{ binds: { directives: "eslint" } }, "binds.directives", "directives reads ESLint's /* exported */ comments; PHP exports no variables; blade has none."],
    [{ binds: { form: "import" } }, "binds.form", "import is not a blade binding form; expected one of assignment, augmented, for, except, parameter, global, static."],
    [{ binds: { redefines: { sameImport: true } } }, "binds.redefines.sameImport", "Qualified import names are Python's; a blade variable is never bound by an import."],
    [{ value: { type: "str" } }, "value.type.0", "str is not a blade type name; expected one of int, float, string, bool, null, array."],
    [{ arguments: { signature: ["a", "/", "b"], bind: { A: "a" } } }, "arguments.signature", "/ and * mark Python's positional-only and keyword-only slots; PHP has no positional-only or keyword-only parameters."],
    [{ arguments: { signature: ["a"], bind: { A: "a" }, mode: "estree" } }, "arguments.mode", "estree reads ESLint's node.arguments of ESTree, JavaScript's syntax tree; blade has none (use language or syntactic)."],
    [{ pattern: "$A == $B", compare: { left: "A", op: "eq", right: "B", text: "structure-parts" } }, "compare.text", "structure-parts compares Python's implicitly concatenated string parts; blade embeds PHP, which has no implicit string concatenation."],
    [{ kind: "string", parse: { grammar: "percentFormat", kind: "placeholder" } }, "parse.grammar", "percentFormat parses Python literals; blade embeds PHP, whose printf formats are printfFormat."],
    [{ kind: "string", parse: { grammar: "braceFormat", kind: "field" } }, "parse.grammar", "braceFormat parses Python literals; blade embeds PHP, which has no str.format templates (its printf formats are printfFormat)."],
    [{ kind: "string", parse: { grammar: "annotation", kind: "identifier" } }, "parse.grammar", "annotation parses Python literals; blade embeds PHP, which holds no type in a string: its declared types are syntax, and its PHPDoc types are phpDoc."],
    [{ kind: "comment", parse: { grammar: "eslintDirective", kind: "directive" } }, "parse.grammar", "eslintDirective parses TypeScript comments."],
    [{ pattern: "sprintf($F, $$$A)", formatArgs: { style: "percent", string: "F", args: "A", missing: { kind: "placeholder" } } }, "formatArgs.style", "percent reads Python's % formats; blade embeds PHP, whose printf-family calls are style printf."],
    [{ pattern: "sprintf($F, $$$A)", formatArgs: { style: "brace", string: "F", args: "A", missing: { kind: "field" } } }, "formatArgs.style", "brace reads Python's str.format calls; blade embeds PHP, which has no such templates (its printf-family calls are style printf)."],
  ])("refuses the Blade rule %j at %s", (rule, at, message) => {
    const issues = codeRuleSchema.safeParse(astRule("blade", rule)).error?.issues ?? [];
    expect(issues.map(issue => ({ path: issue.path.slice(4).join("."), message: issue.message }))).toContainEqual({ path: `rule.${at}`, message });
  });

  it("refuses Blade language options and every linter's suppressions", () => {
    const withOptions = astRule("blade", { kind: "echo_statement" }, { languageOptions: { ecmaVersion: 2020 } });
    expect(codeRuleSchema.safeParse(withOptions).error?.message).toContain("languageOptions: `languageOptions` is not available for blade policies");
    const withPhpstan = astRule("blade", { kind: "echo_statement" }, { suppress: { phpstan: { identifiers: ["variable.undefined"] } } });
    expect(codeRuleSchema.safeParse(withPhpstan).error?.message).toContain("@phpstan-ignore comments are PHPStan's; a blade query has none.");
  });
});
