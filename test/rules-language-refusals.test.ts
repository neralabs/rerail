import { describe, expect, it } from "vitest";
import { validateCodeRule } from "../src/rules.js";

/**
 * Every refusal that depends on a rule's language: a key, value or option one language family lacks, a grammar or
 * format style another family's, a suppression comment another linter's. The schema checks shapes only; each of these
 * is the engine's to refuse, and `validateCodeRule` reports it as invalid-rule with the engine's reason.
 */

type Json = Record<string, unknown>;

const document = (language: string, rule: unknown, ast: Json = {}, violation: Json = {}): Json => ({
  format: "rerail.policy/v1",
  id: "test:refusal",
  revision: 1,
  mode: "steer",
  clauses: [{ id: "check", violation: { code: "TEST", anchor: "witness.span", message: "Test", ...violation }, forbid: { ast: { language, rule, ...ast } } }],
});
const withDefine = (language: string, util: Json, rule: Json): Json => ({
  format: "rerail.policy/v1",
  id: "test:refusal",
  revision: 1,
  mode: "steer",
  clauses: [
    { id: "shared", define: { language, utils: { u: util } } },
    { id: "q", violation: { code: "TEST", anchor: "witness.span", message: "Test" }, forbid: { ast: { language, rule } } },
  ],
});

const ident = (rule: Json) => ({ kind: "identifier", ...rule });
const variable = (rule: Json) => ({ kind: "variable_name", ...rule });
const ECMA = ["typescript", "javascript"] as const;
const PHP = ["php", "blade"] as const;
const nameKind = (language: string) => ((PHP as readonly string[]).includes(language) ? "name" : "identifier");

const cases: [string, Json][] = [];
const refuses = (name: string, rule: Json) => cases.push([name, rule]);

// Capabilities the registry refuses a language.
for (const language of ECMA) {
  refuses(`${language} calls`, document(language, { kind: "call_expression", calls: { kind: "call_expression" } }));
  refuses(`${language} formatArgs`, document(language, { pattern: "f($S, $$$A)", formatArgs: { style: "brace", string: "S", args: "A", unused: { kind: "identifier" } } }));
}
refuses("typescript define utility formatArgs", withDefine("typescript", { kind: "call", formatArgs: { style: "brace", string: "S", args: "A", unused: { kind: "identifier" } } }, { kind: "identifier" }));

// Binding and import keys.
refuses("python resolves.directives", document("python", ident({ resolves: { global: ["a"], directives: "eslint" } })));
refuses("python unresolved directives", document("python", ident({ resolves: { unresolved: true, directives: "eslint" } })));
for (const language of ECMA) {
  refuses(`${language} resolves.import`, document(language, ident({ resolves: { import: ["react.useState"] } })));
  refuses(`${language} resolves.imported`, document(language, ident({ resolves: { imported: ["react"] } })));
  refuses(`${language} resolves.binding.latest`, document(language, ident({ resolves: { binding: { latest: true } } })));
  refuses(`${language} resolves.binding.outer`, document(language, ident({ resolves: { binding: { outer: false } } })));
  refuses(`${language} binds.latest`, document(language, ident({ binds: { latest: true } })));
  refuses(`${language} binds.outer`, document(language, ident({ binds: { outer: false } })));
  refuses(`${language} binds.redefines.sameImport`, document(language, ident({ binds: { redefines: { sameImport: true } } })));
}
refuses("python binds.redefines.every", document("python", ident({ binds: { redefines: { every: true } } })));
for (const language of PHP) refuses(`${language} binds.redefines.sameImport`, document(language, variable({ binds: { redefines: { sameImport: true } } })));

// Resolution, argument and compare keys.
for (const language of ECMA) {
  refuses(`${language} resolves.beforeLocal`, document(language, ident({ resolves: { beforeLocal: true } })));
  refuses(`${language} resolves.unresolved star`, document(language, ident({ resolves: { unresolved: "star" } })));
  refuses(`${language} scriptDeclarations without languageOptions`, document(language, ident({ resolves: { global: ["a"], scriptDeclarations: true } })));
  refuses(`${language} declared without languageOptions`, document(language, ident({ resolves: { global: ["a"], declared: true } })));
  refuses(`${language} this without languageOptions`, document(language, ident({ resolves: { this: "global" } })));
  refuses(`${language} resolves.binding.stopAtGlobal`, document(language, ident({ resolves: { binding: { stopAtGlobal: true } } })));
  refuses(`${language} arguments signature /`, document(language, { kind: "call_expression", arguments: { signature: ["a", "/", "b"], bind: { A: "a" } } }));
  refuses(`${language} arguments signature *`, document(language, { kind: "call_expression", arguments: { signature: ["a", "*", "b"], bind: { A: "a" } } }));
  refuses(`${language} arguments mode syntactic`, document(language, { kind: "call_expression", arguments: { signature: ["a"], bind: { A: "a" }, mode: "syntactic" } }));
  for (const text of ["structure", "structure-hash", "structure-parts"]) {
    refuses(`${language} compare ${text}`, document(language, { pattern: "f($A, $B)", compare: { left: "A", op: "eq", right: "B", text } }));
  }
}
refuses("python resolves.this", document("python", ident({ resolves: { this: "global" } })));
refuses("python resolves.scriptDeclarations", document("python", ident({ resolves: { global: ["a"], scriptDeclarations: true } })));
refuses("python declared without languageOptions", document("python", ident({ resolves: { global: ["a"], declared: true } })));
refuses("python resolves.binding.destructuredValue", document("python", ident({ resolves: { binding: { destructuredValue: false } } })));
refuses("python arguments mode estree", document("python", { kind: "call", arguments: { signature: ["a"], bind: { A: "a" }, mode: "estree" } }));
for (const text of ["key", "member", "reference", "reference-no-static-keys", "name"]) {
  refuses(`python compare ${text}`, document("python", { pattern: "f($A, $B)", compare: { left: "A", op: "eq", right: "B", text } }));
}
for (const language of PHP) {
  refuses(`${language} resolves.binding.stopAtGlobal false`, document(language, variable({ resolves: { binding: { stopAtGlobal: false } } })));
  refuses(`${language} arguments signature /`, document(language, { kind: "function_call_expression", arguments: { signature: ["a", "/", "b"], bind: { A: "a" } } }));
  refuses(`${language} arguments signature *`, document(language, { kind: "function_call_expression", arguments: { signature: ["a", "*", "b"], bind: { A: "a" } } }));
  refuses(`${language} arguments mode estree`, document(language, { kind: "function_call_expression", arguments: { signature: ["a"], bind: { A: "a" }, mode: "estree" } }));
  refuses(`${language} compare structure-parts`, document(language, { pattern: "$A == $B", compare: { left: "A", op: "eq", right: "B", text: "structure-parts" } }));
}

// PHP's resolves: names or variables, superglobals, and no counterpart for the other families' terms.
for (const language of PHP) {
  refuses(`${language} resolves.this`, document(language, variable({ resolves: { this: "global" } })));
  refuses(`${language} resolves.sloppyFunctions`, document(language, variable({ resolves: { this: "global", sloppyFunctions: true } })));
  refuses(`${language} resolves.directives`, document(language, variable({ resolves: { global: ["_GET"], directives: "eslint" } })));
  refuses(`${language} resolves.declared`, document(language, variable({ resolves: { global: ["_GET"], declared: true } })));
  refuses(`${language} resolves.scriptDeclarations`, document(language, variable({ resolves: { global: ["_GET"], scriptDeclarations: true } })));
  refuses(`${language} resolves.relative`, document(language, { kind: "name", resolves: { imported: ["App"], relative: "never" } }));
  refuses(`${language} resolves.seenBefore`, document(language, { kind: "name", resolves: { imported: ["Foo"], seenBefore: true } }));
  refuses(`${language} resolves.beforeLocal`, document(language, variable({ resolves: { beforeLocal: true } })));
  refuses(`${language} names beside variables`, document(language, variable({ resolves: { imported: ["exec"], global: ["_GET"] } })));
  refuses(`${language} unresolved star`, document(language, variable({ resolves: { unresolved: "star" } })));
  refuses(`${language} global not a superglobal`, document(language, variable({ resolves: { global: ["_GET", "argv"] } })));
}

// Import names in the other family's shape.
refuses("python import of a PHP name", document("python", ident({ resolves: { import: ["App\\Foo"] } })));
refuses("python imported PHP name", document("python", { kind: "dotted_name", resolves: { imported: ["App\\Foo"] } }));
for (const language of PHP) {
  refuses(`${language} dotted import`, document(language, { kind: "name", resolves: { import: ["os.path"] } }));
  refuses(`${language} dotted imported`, document(language, { kind: "name", resolves: { imported: ["os.path"] } }));
}

// Where ESLint reports a variable.
refuses("python resolves.write", document("python", ident({ resolves: { write: "last" } })));
for (const language of PHP) refuses(`${language} resolves.write`, document(language, variable({ resolves: { write: "last", binding: {} } })));

// Binding filters: forms, scope kinds and keys of another family.
for (const language of PHP) {
  refuses(`${language} binds.form import`, document(language, variable({ binds: { form: "import" } })));
  refuses(`${language} binds.scope module`, document(language, variable({ binds: { scope: ["function", "module"] } })));
  refuses(`${language} binds.exported`, document(language, variable({ binds: { exported: true } })));
  refuses(`${language} binds.builtin`, document(language, variable({ binds: { builtin: true } })));
  refuses(`${language} binds.directives`, document(language, variable({ binds: { directives: "eslint" } })));
  refuses(`${language} resolves.binding.exported`, document(language, variable({ resolves: { binding: { exported: true } } })));
}
for (const language of ["python", ...ECMA]) refuses(`${language} binds.form static`, document(language, ident({ binds: { form: "static" } })));
refuses("python binds.scope block", document("python", ident({ binds: { scope: ["module", "block"] } })));
refuses("python binds.scope closure", document("python", ident({ binds: { scope: "closure" } })));
for (const language of ECMA) {
  refuses(`${language} binds.scope lambda`, document(language, ident({ binds: { scope: "lambda" } })));
  refuses(`${language} binds.scope closure`, document(language, ident({ binds: { scope: "closure" } })));
}
refuses("python binds.directives", document("python", ident({ binds: { directives: "eslint" } })));
refuses("python binds.builtin", document("python", ident({ binds: { builtin: true } })));
refuses("python binds.redefines.builtin", document("python", ident({ binds: { redefines: { builtin: true } } })));
refuses("python resolves.binding.form static", document("python", ident({ resolves: { binding: { form: "static" } } })));
refuses("python define utility binds.builtin", withDefine("python", { kind: "identifier", binds: { builtin: true } }, { matches: "u" }));
refuses("python constraint binds.builtin", document("python", { pattern: "f($A)" }, { constraints: { A: { binds: { builtin: true } } } }));

// A variable's occurrences.
refuses("python binds.occurrence", document("python", ident({ binds: { occurrence: { kind: "identifier" } } })));
refuses("python resolves.binding.occurrence", document("python", ident({ resolves: { binding: { occurrence: { kind: "identifier" } } } })));

// Value type names.
refuses("python value.type string", document("python", { kind: "string", value: { type: "string" } }));
for (const language of ECMA) refuses(`${language} value.type str`, document(language, { kind: "string", value: { type: "str" } }));
for (const language of PHP) refuses(`${language} value.type str`, document(language, { kind: "string", value: { type: ["string", "str"] } }));
refuses("typescript binding value type str", document("typescript", ident({ resolves: { binding: { value: { value: { type: "str" } } } } })));
refuses("php query utility value.type str", document("php", { matches: "u" }, { utils: { u: { kind: "string", value: { type: "str" } } } }));

// Grammars whose literals are another family's.
const PARSED_KIND: Record<string, string> = { percentFormat: "placeholder", braceFormat: "field", annotation: "identifier", eslintDirective: "directive", printfFormat: "placeholder", scanfFormat: "placeholder", phpDoc: "php_doc" };
const FOREIGN_GRAMMARS: Record<string, readonly string[]> = {
  python: ["eslintDirective", "printfFormat", "scanfFormat", "phpDoc"],
  typescript: ["percentFormat", "braceFormat", "annotation", "printfFormat", "scanfFormat", "phpDoc"],
  javascript: ["percentFormat", "braceFormat", "annotation", "printfFormat", "scanfFormat", "phpDoc"],
  php: ["percentFormat", "braceFormat", "annotation", "eslintDirective"],
  blade: ["percentFormat", "braceFormat", "annotation", "eslintDirective"],
};
for (const [language, grammars] of Object.entries(FOREIGN_GRAMMARS)) {
  for (const grammar of grammars) {
    const host = grammar === "eslintDirective" || grammar === "phpDoc" ? "comment" : "string";
    refuses(`${language} parse ${grammar}`, document(language, { kind: host, parse: { grammar, kind: PARSED_KIND[grammar] } }));
  }
}

// Format styles of another family.
const style = (name: string, missing: string) => ({ pattern: "f($F, $$$A)", formatArgs: { style: name, string: "F", args: "A", missing: { kind: missing } } });
refuses("python style printf", document("python", style("printf", "placeholder")));
refuses("python style scanf", document("python", style("scanf", "placeholder")));
for (const language of PHP) {
  refuses(`${language} style percent`, document(language, style("percent", "placeholder")));
  refuses(`${language} style brace`, document(language, style("brace", "field")));
}

// Language options, version gates and file conditions of another family.
for (const language of PHP) refuses(`${language} languageOptions`, document(language, { kind: "name" }, { languageOptions: { ecmaVersion: 2020 } }));
const ECMA_OPTIONS: Json = { ecmaVersion: 2020, sourceType: "script", globals: { a: "readonly" }, parserOptions: { ecmaFeatures: { globalReturn: true } }, jsxPragma: "h", jsxFragmentName: "F", lib: ["es2020"] };
for (const [key, value] of Object.entries(ECMA_OPTIONS)) refuses(`python languageOptions.${key}`, document("python", ident({}), { languageOptions: { [key]: value } }));
const PYTHON_OPTIONS: Json = { pythonVersion: "3.12", typingModules: ["foo"], extendGenerics: ["foo.Bar"] };
for (const language of ECMA) {
  for (const [key, value] of Object.entries(PYTHON_OPTIONS)) refuses(`${language} languageOptions.${key}`, document(language, ident({}), { languageOptions: { [key]: value } }));
}
const ECMA_GATES: Json = { ecmaVersion: { min: 2015 }, sourceType: "module", ecmaFeatures: { globalReturn: true } };
for (const [key, value] of Object.entries(ECMA_GATES)) refuses(`python when.${key}`, document("python", ident({}), { when: { [key]: value } }));
for (const language of [...ECMA, ...PHP]) refuses(`${language} when.pythonVersion`, document(language, { kind: nameKind(language) }, { when: { pythonVersion: { min: "3.8" } } }));
const MODULE_CONDITIONS: Json = { module: { regex: "^a" }, moduleName: { regex: "^a" }, package: { regex: "^a" }, inPackage: true };
for (const language of [...ECMA, ...PHP]) {
  for (const [key, value] of Object.entries(MODULE_CONDITIONS)) refuses(`${language} file.${key}`, document(language, { kind: nameKind(language) }, { file: { [key]: value } }));
}
refuses("python file.declaration", document("python", ident({}), { file: { declaration: true } }));

// Suppression comments of a linter the language has none of.
const SUPPRESSIONS: Record<string, Json> = { eslint: { rules: ["strict"] }, noqa: { codes: ["F401"] }, phpstan: { identifiers: ["variable.undefined"] }, psalm: { issues: ["UndefinedVariable"] }, phpcs: { sniffs: ["Squiz.PHP.DiscouragedFunctions"] } };
const OWN_SUPPRESSIONS: Record<string, readonly string[]> = { python: ["noqa"], typescript: ["eslint"], javascript: ["eslint"], php: ["phpstan", "psalm", "phpcs"], blade: [] };
for (const [language, own] of Object.entries(OWN_SUPPRESSIONS)) {
  for (const [linter, suppress] of Object.entries(SUPPRESSIONS)) {
    if (!own.includes(linter)) refuses(`${language} suppress.${linter}`, document(language, { kind: nameKind(language) }, {}, { suppress: { [linter]: suppress } }));
  }
}

/** The engine's reason for each case, recorded from the engine's own compile before the host stopped mirroring it. */
const ENGINE_REASONS: Record<string, string> = {
  "typescript calls": "/clauses/0/forbid/ast/rule/calls: `calls` is not available for typescript policies",
  "typescript formatArgs": "/clauses/0/forbid/ast/rule/formatArgs: `formatArgs` is not available for typescript policies",
  "javascript calls": "/clauses/0/forbid/ast/rule/calls: `calls` is not available for javascript policies",
  "javascript formatArgs": "/clauses/0/forbid/ast/rule/formatArgs: `formatArgs` is not available for javascript policies",
  "python resolves.directives": "/clauses/0/forbid/ast/rule/resolves/directives: /* global */ comments are ESLint's; Python has none",
  "python unresolved directives": "/clauses/0/forbid/ast/rule/resolves/directives: /* global */ comments are ESLint's; Python has none",
  "typescript resolves.import": "/clauses/0/forbid/ast/rule/resolves/import: import resolution is Python-only for now; TypeScript rules can use `global`",
  "typescript resolves.imported": "/clauses/0/forbid/ast/rule/resolves/imported: import resolution is Python-only for now; TypeScript rules can use `global`",
  "typescript resolves.binding.latest": "/clauses/0/forbid/ast/rule/resolves/binding/latest: `latest` reads Python's binding order and declarations; TypeScript has none",
  "typescript resolves.binding.outer": "/clauses/0/forbid/ast/rule/resolves/binding/outer: `outer` reads Python's binding order and declarations; TypeScript has none",
  "typescript binds.latest": "/clauses/0/forbid/ast/rule/binds/latest: `latest` reads Python's binding order and declarations; TypeScript has none",
  "typescript binds.outer": "/clauses/0/forbid/ast/rule/binds/outer: `outer` reads Python's binding order and declarations; TypeScript has none",
  "typescript binds.redefines.sameImport": "/clauses/0/forbid/ast/rule/binds/redefines/sameImport: qualified import names are Python's; TypeScript has none",
  "javascript resolves.import": "/clauses/0/forbid/ast/rule/resolves/import: import resolution is Python-only for now; TypeScript rules can use `global`",
  "javascript resolves.imported": "/clauses/0/forbid/ast/rule/resolves/imported: import resolution is Python-only for now; TypeScript rules can use `global`",
  "javascript resolves.binding.latest": "/clauses/0/forbid/ast/rule/resolves/binding/latest: `latest` reads Python's binding order and declarations; TypeScript has none",
  "javascript resolves.binding.outer": "/clauses/0/forbid/ast/rule/resolves/binding/outer: `outer` reads Python's binding order and declarations; TypeScript has none",
  "javascript binds.latest": "/clauses/0/forbid/ast/rule/binds/latest: `latest` reads Python's binding order and declarations; TypeScript has none",
  "javascript binds.outer": "/clauses/0/forbid/ast/rule/binds/outer: `outer` reads Python's binding order and declarations; TypeScript has none",
  "javascript binds.redefines.sameImport": "/clauses/0/forbid/ast/rule/binds/redefines/sameImport: qualified import names are Python's; TypeScript has none",
  "python binds.redefines.every": "/clauses/0/forbid/ast/rule/binds/redefines/every: Python's redefines reads the one binding shadowed; `every` is TypeScript's",
  "php binds.redefines.sameImport": "/clauses/0/forbid/ast/rule/binds/redefines/sameImport: qualified import names are Python's; a PHP variable is never bound by an import",
  "blade binds.redefines.sameImport": "/clauses/0/forbid/ast/rule/binds/redefines/sameImport: qualified import names are Python's; a blade variable is never bound by an import",
  "typescript resolves.beforeLocal": "/clauses/0/forbid/ast/rule/resolves/beforeLocal reads Python's binding order; TypeScript has none",
  "typescript resolves.unresolved star": "/clauses/0/forbid/ast/rule/resolves/unresolved: \"star\" asks whether a Python star import binds the name; TypeScript has none",
  "typescript scriptDeclarations without languageOptions": "/clauses/0/forbid/ast/rule/resolves/scriptDeclarations: whether a file is a script comes from languageOptions.sourceType; declare languageOptions",
  "typescript declared without languageOptions": "/clauses/0/forbid/ast/rule/resolves/declared: the declared globals come from languageOptions; declare languageOptions",
  "typescript this without languageOptions": "/clauses/0/forbid/ast/rule/resolves/this: the global object depends on languageOptions.sourceType; declare languageOptions",
  "typescript resolves.binding.stopAtGlobal": "/clauses/0/forbid/ast/rule/resolves/binding/stopAtGlobal: `global` statements are Python's; TypeScript has none",
  "typescript arguments signature /": "/clauses/0/forbid/ast/rule/arguments/signature: `/` and `*` mark Python's positional-only and keyword-only slots; JavaScript passes arguments by position only",
  "typescript arguments signature *": "/clauses/0/forbid/ast/rule/arguments/signature: `/` and `*` mark Python's positional-only and keyword-only slots; JavaScript passes arguments by position only",
  "typescript arguments mode syntactic": "/clauses/0/forbid/ast/rule/arguments/mode: `syntactic` finds arguments by position and keyword as Ruff's find_argument does; TypeScript has no keywords",
  "typescript compare structure": "/clauses/0/forbid/ast/rule/compare/text: the structural modes compare Python and PHP syntax; use tokens for TypeScript",
  "typescript compare structure-hash": "/clauses/0/forbid/ast/rule/compare/text: the structural modes compare Python and PHP syntax; use tokens for TypeScript",
  "typescript compare structure-parts": "/clauses/0/forbid/ast/rule/compare/text: the structural modes compare Python and PHP syntax; use tokens for TypeScript",
  "javascript resolves.beforeLocal": "/clauses/0/forbid/ast/rule/resolves/beforeLocal reads Python's binding order; TypeScript has none",
  "javascript resolves.unresolved star": "/clauses/0/forbid/ast/rule/resolves/unresolved: \"star\" asks whether a Python star import binds the name; TypeScript has none",
  "javascript scriptDeclarations without languageOptions": "/clauses/0/forbid/ast/rule/resolves/scriptDeclarations: whether a file is a script comes from languageOptions.sourceType; declare languageOptions",
  "javascript declared without languageOptions": "/clauses/0/forbid/ast/rule/resolves/declared: the declared globals come from languageOptions; declare languageOptions",
  "javascript this without languageOptions": "/clauses/0/forbid/ast/rule/resolves/this: the global object depends on languageOptions.sourceType; declare languageOptions",
  "javascript resolves.binding.stopAtGlobal": "/clauses/0/forbid/ast/rule/resolves/binding/stopAtGlobal: `global` statements are Python's; TypeScript has none",
  "javascript arguments signature /": "/clauses/0/forbid/ast/rule/arguments/signature: `/` and `*` mark Python's positional-only and keyword-only slots; JavaScript passes arguments by position only",
  "javascript arguments signature *": "/clauses/0/forbid/ast/rule/arguments/signature: `/` and `*` mark Python's positional-only and keyword-only slots; JavaScript passes arguments by position only",
  "javascript arguments mode syntactic": "/clauses/0/forbid/ast/rule/arguments/mode: `syntactic` finds arguments by position and keyword as Ruff's find_argument does; TypeScript has no keywords",
  "javascript compare structure": "/clauses/0/forbid/ast/rule/compare/text: the structural modes compare Python and PHP syntax; use tokens for TypeScript",
  "javascript compare structure-hash": "/clauses/0/forbid/ast/rule/compare/text: the structural modes compare Python and PHP syntax; use tokens for TypeScript",
  "javascript compare structure-parts": "/clauses/0/forbid/ast/rule/compare/text: the structural modes compare Python and PHP syntax; use tokens for TypeScript",
  "python resolves.this": "/clauses/0/forbid/ast/rule/resolves/this: `this` is TypeScript's; Python has none",
  "python resolves.scriptDeclarations": "/clauses/0/forbid/ast/rule/resolves/scriptDeclarations: scripts are ESLint's; Python has none",
  "python declared without languageOptions": "/clauses/0/forbid/ast/rule/resolves/declared: the declared globals come from languageOptions; declare languageOptions",
  "python resolves.binding.destructuredValue": "/clauses/0/forbid/ast/rule/resolves/binding/destructuredValue: destructured values are TypeScript's; a Python unpacked name keeps its element",
  "python arguments mode estree": "/clauses/0/forbid/ast/rule/arguments/mode: `estree` reads ESLint's `node.arguments`; use language or syntactic for Python",
  "python compare key": "/clauses/0/forbid/ast/rule/compare/text: `key` reads JavaScript and PHP property names; Python has none",
  "python compare member": "/clauses/0/forbid/ast/rule/compare/text: `member` reads JavaScript and PHP property names; Python has none",
  "python compare reference": "/clauses/0/forbid/ast/rule/compare/text: `reference` reads JavaScript and PHP references (ESLint's isSameReference); use structure for Python",
  "python compare reference-no-static-keys": "/clauses/0/forbid/ast/rule/compare/text: `reference-no-static-keys` reads JavaScript and PHP references (ESLint's isSameReference); use structure for Python",
  "python compare name": "/clauses/0/forbid/ast/rule/compare/text: `name` reads the names JavaScript and PHP identifiers and strings write; use unquoted for Python",
  "php resolves.binding.stopAtGlobal false": "/clauses/0/forbid/ast/rule/resolves/binding/stopAtGlobal: a php read after `global $x` reads that statement's binding; which top-level binding the global variable then holds depends on when the function runs, so php has no lookup through it",
  "php arguments signature /": "/clauses/0/forbid/ast/rule/arguments/signature: `/` and `*` mark Python's positional-only and keyword-only slots; PHP has no positional-only or keyword-only parameters",
  "php arguments signature *": "/clauses/0/forbid/ast/rule/arguments/signature: `/` and `*` mark Python's positional-only and keyword-only slots; PHP has no positional-only or keyword-only parameters",
  "php arguments mode estree": "/clauses/0/forbid/ast/rule/arguments/mode: `estree` reads ESLint's `node.arguments` of ESTree, JavaScript's syntax tree; php has none (use language or syntactic)",
  "php compare structure-parts": "/clauses/0/forbid/ast/rule/compare/text: `structure-parts` compares Python's implicitly concatenated string parts; PHP has no implicit string concatenation",
  "blade resolves.binding.stopAtGlobal false": "/clauses/0/forbid/ast/rule/resolves/binding/stopAtGlobal: a blade read after `global $x` reads that statement's binding; which top-level binding the global variable then holds depends on when the function runs, so blade has no lookup through it",
  "blade arguments signature /": "/clauses/0/forbid/ast/rule/arguments/signature: `/` and `*` mark Python's positional-only and keyword-only slots; PHP has no positional-only or keyword-only parameters",
  "blade arguments signature *": "/clauses/0/forbid/ast/rule/arguments/signature: `/` and `*` mark Python's positional-only and keyword-only slots; PHP has no positional-only or keyword-only parameters",
  "blade arguments mode estree": "/clauses/0/forbid/ast/rule/arguments/mode: `estree` reads ESLint's `node.arguments` of ESTree, JavaScript's syntax tree; blade has none (use language or syntactic)",
  "blade compare structure-parts": "/clauses/0/forbid/ast/rule/compare/text: `structure-parts` compares Python's implicitly concatenated string parts; blade embeds PHP, which has no implicit string concatenation",
  "php resolves.this": "/clauses/0/forbid/ast/rule/resolves/this: `this` asks whether JavaScript's `this` is the global object; PHP's `$this` never is: outside an object context it is undefined, which `unresolved: true` reports; php has none",
  "php resolves.sloppyFunctions": "/clauses/0/forbid/ast/rule/resolves/this: `this` asks whether JavaScript's `this` is the global object; PHP's `$this` never is: outside an object context it is undefined, which `unresolved: true` reports; php has none",
  "php resolves.directives": "/clauses/0/forbid/ast/rule/resolves/directives: /* global */ comments are ESLint's; PHP's superglobals are fixed, and a function declares a global with a `global` statement, which `binding` reads; php has none",
  "php resolves.declared": "/clauses/0/forbid/ast/rule/resolves/declared: declared globals come from ESLint's and Ruff's configuration; PHP's superglobals are always declared, and no languageOptions configure others; php has none",
  "php resolves.scriptDeclarations": "/clauses/0/forbid/ast/rule/resolves/scriptDeclarations: scripts are ESLint's; a PHP file has no script mode, and a superglobal is never a file's declaration; php has none",
  "php resolves.relative": "/clauses/0/forbid/ast/rule/resolves/relative: a `use` statement names what it imports fully qualified (`use namespace\\A;` does not parse); `import` resolves a `namespace\\A` reference; php has none",
  "php resolves.seenBefore": "/clauses/0/forbid/ast/rule/resolves/seenBefore: Ruff's `seen_module` asks whether a module was imported before; a PHP `use` only aliases a name for the rest of its namespace and loads nothing; php has none",
  "php resolves.beforeLocal": "/clauses/0/forbid/ast/rule/resolves/beforeLocal: a PHP function sees no variable of an enclosing scope, so a read before the function binds the name is undefined, which `unresolved: true` reports; php has none",
  "php names beside variables": "/clauses/0/forbid/ast/rule/resolves: `import` and `imported` resolve class, function and constant names; `global` and `unresolved` resolve variables; one `resolves` asks one of them",
  "php unresolved star": "/clauses/0/forbid/ast/rule/resolves/unresolved: php policies ask `unresolved: true`",
  "php global not a superglobal": "/clauses/0/forbid/ast/rule/resolves/global/1: `argv` is no PHP superglobal; expected one of GLOBALS, _SERVER, _GET, _POST, _FILES, _COOKIE, _SESSION, _REQUEST, _ENV",
  "blade resolves.this": "/clauses/0/forbid/ast/rule/resolves/this: `this` asks whether JavaScript's `this` is the global object; PHP's `$this` never is: outside an object context it is undefined, which `unresolved: true` reports; blade has none",
  "blade resolves.sloppyFunctions": "/clauses/0/forbid/ast/rule/resolves/this: `this` asks whether JavaScript's `this` is the global object; PHP's `$this` never is: outside an object context it is undefined, which `unresolved: true` reports; blade has none",
  "blade resolves.directives": "/clauses/0/forbid/ast/rule/resolves/directives: /* global */ comments are ESLint's; PHP's superglobals are fixed, and a function declares a global with a `global` statement, which `binding` reads; blade has none",
  "blade resolves.declared": "/clauses/0/forbid/ast/rule/resolves/declared: declared globals come from ESLint's and Ruff's configuration; PHP's superglobals are always declared, and no languageOptions configure others; blade has none",
  "blade resolves.scriptDeclarations": "/clauses/0/forbid/ast/rule/resolves/scriptDeclarations: scripts are ESLint's; a PHP file has no script mode, and a superglobal is never a file's declaration; blade has none",
  "blade resolves.relative": "/clauses/0/forbid/ast/rule/resolves/relative: a `use` statement names what it imports fully qualified (`use namespace\\A;` does not parse); `import` resolves a `namespace\\A` reference; blade has none",
  "blade resolves.seenBefore": "/clauses/0/forbid/ast/rule/resolves/seenBefore: Ruff's `seen_module` asks whether a module was imported before; a PHP `use` only aliases a name for the rest of its namespace and loads nothing; blade has none",
  "blade resolves.beforeLocal": "/clauses/0/forbid/ast/rule/resolves/beforeLocal: a PHP function sees no variable of an enclosing scope, so a read before the function binds the name is undefined, which `unresolved: true` reports; blade has none",
  "blade names beside variables": "/clauses/0/forbid/ast/rule/resolves: `import` and `imported` resolve class, function and constant names; `global` and `unresolved` resolve variables; one `resolves` asks one of them",
  "blade unresolved star": "/clauses/0/forbid/ast/rule/resolves/unresolved: blade policies ask `unresolved: true`",
  "blade global not a superglobal": "/clauses/0/forbid/ast/rule/resolves/global/1: `argv` is no PHP superglobal; expected one of GLOBALS, _SERVER, _GET, _POST, _FILES, _COOKIE, _SESSION, _REQUEST, _ENV",
  "python import of a PHP name": "/clauses/0/forbid/ast/rule/resolves/import/0: `App\\Foo` is a PHP name; Python imports are dotted names",
  "python imported PHP name": "/clauses/0/forbid/ast/rule/resolves/imported/0: `App\\Foo` is a PHP name; Python imports are dotted names",
  "php dotted import": "/clauses/0/forbid/ast/rule/resolves/import/0: `os.path` is a dotted name; PHP names join labels with `\\`",
  "php dotted imported": "/clauses/0/forbid/ast/rule/resolves/imported/0: `os.path` is a dotted name; PHP names join labels with `\\`",
  "blade dotted import": "/clauses/0/forbid/ast/rule/resolves/import/0: `os.path` is a dotted name; PHP names join labels with `\\`",
  "blade dotted imported": "/clauses/0/forbid/ast/rule/resolves/imported/0: `os.path` is a dotted name; PHP names join labels with `\\`",
  "python resolves.write": "/clauses/0/forbid/ast/rule/resolves/write: where ESLint reports a variable is TypeScript's; Python has none",
  "php resolves.write": "/clauses/0/forbid/ast/rule/resolves/write: ESLint reports an unused variable once, at its last write; php's usage oracle, Psalm, reports each unused assignment, which `binds: { used: false }` matches",
  "blade resolves.write": "/clauses/0/forbid/ast/rule/resolves/write: ESLint reports an unused variable once, at its last write; blade reads its embedded PHP, whose usage oracle, Psalm, reports each unused assignment, which `binds: { used: false }` matches",
  "php binds.form import": "/clauses/0/forbid/ast/rule/binds/form: `\"import\"` is not a binding form; expected one of assignment, augmented, for, except, parameter, global, static",
  "php binds.scope module": "/clauses/0/forbid/ast/rule/binds/scope: `\"module\"` is not a scope kind; expected one of global, function, closure",
  "php binds.exported": "/clauses/0/forbid/ast/rule/binds/exported: `exported` reads Python's re-exports and TypeScript's exports; a PHP file exports no variables (an including file shares its global scope, which no declaration exports); php has none",
  "php binds.builtin": "/clauses/0/forbid/ast/rule/binds/builtin: `builtin` reads ESLint's configured globals; PHP's superglobals are no file's bindings (an assignment to `$_GET` writes the superglobal); php has none",
  "php binds.directives": "/clauses/0/forbid/ast/rule/binds/directives: `directives` reads ESLint's /* exported */ comments; PHP exports no variables; php has none",
  "php resolves.binding.exported": "/clauses/0/forbid/ast/rule/resolves/binding/exported: `exported` reads Python's re-exports and TypeScript's exports; a PHP file exports no variables (an including file shares its global scope, which no declaration exports); php has none",
  "blade binds.form import": "/clauses/0/forbid/ast/rule/binds/form: `\"import\"` is not a binding form; expected one of assignment, augmented, for, except, parameter, global, static",
  "blade binds.scope module": "/clauses/0/forbid/ast/rule/binds/scope: `\"module\"` is not a scope kind; expected one of global, function, closure",
  "blade binds.exported": "/clauses/0/forbid/ast/rule/binds/exported: `exported` reads Python's re-exports and TypeScript's exports; a PHP file exports no variables (an including file shares its global scope, which no declaration exports); blade has none",
  "blade binds.builtin": "/clauses/0/forbid/ast/rule/binds/builtin: `builtin` reads ESLint's configured globals; PHP's superglobals are no file's bindings (an assignment to `$_GET` writes the superglobal); blade has none",
  "blade binds.directives": "/clauses/0/forbid/ast/rule/binds/directives: `directives` reads ESLint's /* exported */ comments; PHP exports no variables; blade has none",
  "blade resolves.binding.exported": "/clauses/0/forbid/ast/rule/resolves/binding/exported: `exported` reads Python's re-exports and TypeScript's exports; a PHP file exports no variables (an including file shares its global scope, which no declaration exports); blade has none",
  "python binds.form static": "/clauses/0/forbid/ast/rule/binds/form: `\"static\"` is not a binding form; expected one of assignment, annotated, annotation, augmented, named, for, with, except, import, definition, parameter, global, match, typeParameter, typeAlias, implicit, deletion, field, method, accessor",
  "typescript binds.form static": "/clauses/0/forbid/ast/rule/binds/form: `\"static\"` is not a binding form; expected one of assignment, annotated, annotation, augmented, named, for, with, except, import, definition, parameter, global, match, typeParameter, typeAlias, implicit, deletion, field, method, accessor",
  "javascript binds.form static": "/clauses/0/forbid/ast/rule/binds/form: `\"static\"` is not a binding form; expected one of assignment, annotated, annotation, augmented, named, for, with, except, import, definition, parameter, global, match, typeParameter, typeAlias, implicit, deletion, field, method, accessor",
  "python binds.scope block": "/clauses/0/forbid/ast/rule/binds/scope: `\"block\"` is not a scope kind; expected one of module, class, function, lambda, comprehension, typeParameters",
  "python binds.scope closure": "/clauses/0/forbid/ast/rule/binds/scope: `\"closure\"` is not a scope kind; expected one of module, class, function, lambda, comprehension, typeParameters",
  "typescript binds.scope lambda": "/clauses/0/forbid/ast/rule/binds/scope: `\"lambda\"` is not a scope kind; expected one of global, module, function, class, block, namespace, typeParameters, functionName",
  "typescript binds.scope closure": "/clauses/0/forbid/ast/rule/binds/scope: `\"closure\"` is not a scope kind; expected one of global, module, function, class, block, namespace, typeParameters, functionName",
  "javascript binds.scope lambda": "/clauses/0/forbid/ast/rule/binds/scope: `\"lambda\"` is not a scope kind; expected one of global, module, function, class, block, namespace, typeParameters, functionName",
  "javascript binds.scope closure": "/clauses/0/forbid/ast/rule/binds/scope: `\"closure\"` is not a scope kind; expected one of global, module, function, class, block, namespace, typeParameters, functionName",
  "python binds.directives": "/clauses/0/forbid/ast/rule/binds/directives: /* exported */ comments are ESLint's; Python has none",
  "python binds.builtin": "/clauses/0/forbid/ast/rule/binds/builtin: configured globals are ESLint's; Python has none",
  "python binds.redefines.builtin": "/clauses/0/forbid/ast/rule/binds/redefines/builtin: configured globals are ESLint's; Python has none",
  "python resolves.binding.form static": "/clauses/0/forbid/ast/rule/resolves/binding/form: `\"static\"` is not a binding form; expected one of assignment, annotated, annotation, augmented, named, for, with, except, import, definition, parameter, global, match, typeParameter, typeAlias, implicit, deletion, field, method, accessor",
  "python binds.occurrence": "/clauses/0/forbid/ast/rule/binds/occurrence: a variable's occurrences are TypeScript's; Python has none",
  "python resolves.binding.occurrence": "/clauses/0/forbid/ast/rule/resolves/binding/occurrence: a variable's occurrences are TypeScript's; Python has none",
  "python value.type string": "/clauses/0/forbid/ast/rule/value/type/0: `string` is not a Python type name; expected one of int, float, complex, str, bytes, bool, NoneType, ellipsis, tuple, list, set, dict",
  "typescript value.type str": "/clauses/0/forbid/ast/rule/value/type/0: `str` is not a TypeScript type name; expected one of string, number, bigint, boolean, null, undefined, array, object, regexp",
  "javascript value.type str": "/clauses/0/forbid/ast/rule/value/type/0: `str` is not a TypeScript type name; expected one of string, number, bigint, boolean, null, undefined, array, object, regexp",
  "php value.type str": "/clauses/0/forbid/ast/rule/value/type/1: `str` is not a PHP type name; expected one of int, float, string, bool, null, array",
  "blade value.type str": "/clauses/0/forbid/ast/rule/value/type/1: `str` is not a PHP type name; expected one of int, float, string, bool, null, array",
  "typescript binding value type str": "/clauses/0/forbid/ast/rule/resolves/binding/value/value/type/0: `str` is not a TypeScript type name; expected one of string, number, bigint, boolean, null, undefined, array, object, regexp",
  "python parse eslintDirective": "/clauses/0/forbid/ast/rule/parse/grammar: `eslintDirective` parses TypeScript comments",
  "python parse printfFormat": "/clauses/0/forbid/ast/rule/parse/grammar: `printfFormat` parses PHP strings; Python's printf formats are `percentFormat`",
  "python parse scanfFormat": "/clauses/0/forbid/ast/rule/parse/grammar: `scanfFormat` parses PHP strings; Python has no scanf formats",
  "python parse phpDoc": "/clauses/0/forbid/ast/rule/parse/grammar: `phpDoc` parses PHP doc comments",
  "typescript parse percentFormat": "/clauses/0/forbid/ast/rule/parse/grammar: `percentFormat` parses Python literals",
  "typescript parse braceFormat": "/clauses/0/forbid/ast/rule/parse/grammar: `braceFormat` parses Python literals",
  "typescript parse annotation": "/clauses/0/forbid/ast/rule/parse/grammar: `annotation` parses Python literals",
  "typescript parse printfFormat": "/clauses/0/forbid/ast/rule/parse/grammar: `printfFormat` parses PHP strings",
  "typescript parse scanfFormat": "/clauses/0/forbid/ast/rule/parse/grammar: `scanfFormat` parses PHP strings",
  "typescript parse phpDoc": "/clauses/0/forbid/ast/rule/parse/grammar: `phpDoc` parses PHP doc comments",
  "javascript parse percentFormat": "/clauses/0/forbid/ast/rule/parse/grammar: `percentFormat` parses Python literals",
  "javascript parse braceFormat": "/clauses/0/forbid/ast/rule/parse/grammar: `braceFormat` parses Python literals",
  "javascript parse annotation": "/clauses/0/forbid/ast/rule/parse/grammar: `annotation` parses Python literals",
  "javascript parse printfFormat": "/clauses/0/forbid/ast/rule/parse/grammar: `printfFormat` parses PHP strings",
  "javascript parse scanfFormat": "/clauses/0/forbid/ast/rule/parse/grammar: `scanfFormat` parses PHP strings",
  "javascript parse phpDoc": "/clauses/0/forbid/ast/rule/parse/grammar: `phpDoc` parses PHP doc comments",
  "php parse percentFormat": "/clauses/0/forbid/ast/rule/parse/grammar: `percentFormat` parses Python literals; PHP's printf formats are `printfFormat`",
  "php parse braceFormat": "/clauses/0/forbid/ast/rule/parse/grammar: `braceFormat` parses Python literals; PHP has no str.format templates (its printf formats are `printfFormat`)",
  "php parse annotation": "/clauses/0/forbid/ast/rule/parse/grammar: `annotation` parses Python literals; PHP holds no type in a string: its declared types are syntax, and its PHPDoc types are `phpDoc`",
  "php parse eslintDirective": "/clauses/0/forbid/ast/rule/parse/grammar: `eslintDirective` parses TypeScript comments",
  "blade parse percentFormat": "/clauses/0/forbid/ast/rule/parse/grammar: `percentFormat` parses Python literals; blade embeds PHP, whose printf formats are `printfFormat`",
  "blade parse braceFormat": "/clauses/0/forbid/ast/rule/parse/grammar: `braceFormat` parses Python literals; blade embeds PHP, which has no str.format templates (its printf formats are `printfFormat`)",
  "blade parse annotation": "/clauses/0/forbid/ast/rule/parse/grammar: `annotation` parses Python literals; blade embeds PHP, which holds no type in a string: its declared types are syntax, and its PHPDoc types are `phpDoc`",
  "blade parse eslintDirective": "/clauses/0/forbid/ast/rule/parse/grammar: `eslintDirective` parses TypeScript comments",
  "python style printf": "/clauses/0/forbid/ast/rule/formatArgs/style: `printf` reads PHP's printf-family calls; Python's printf formats are style `percent`",
  "python style scanf": "/clauses/0/forbid/ast/rule/formatArgs/style: `scanf` reads PHP's sscanf and fscanf calls; Python has no scanf formats",
  "php style percent": "/clauses/0/forbid/ast/rule/formatArgs/style: `percent` reads Python's `%` formats; PHP's printf-family calls are style `printf`",
  "php style brace": "/clauses/0/forbid/ast/rule/formatArgs/style: `brace` reads Python's `str.format` calls; PHP has no such templates (its printf-family calls are style `printf`)",
  "blade style percent": "/clauses/0/forbid/ast/rule/formatArgs/style: `percent` reads Python's `%` formats; blade embeds PHP, whose printf-family calls are style `printf`",
  "blade style brace": "/clauses/0/forbid/ast/rule/formatArgs/style: `brace` reads Python's `str.format` calls; blade embeds PHP, which has no such templates (its printf-family calls are style `printf`)",
  "php languageOptions": "/clauses/0/forbid/ast/languageOptions: `languageOptions` is not available for php policies",
  "blade languageOptions": "/clauses/0/forbid/ast/languageOptions: `languageOptions` is not available for blade policies",
  "python languageOptions.ecmaVersion": "unknown field `ecmaVersion` at /clauses/0/forbid/ast/languageOptions",
  "python languageOptions.sourceType": "unknown field `sourceType` at /clauses/0/forbid/ast/languageOptions",
  "python languageOptions.globals": "unknown field `globals` at /clauses/0/forbid/ast/languageOptions",
  "python languageOptions.parserOptions": "unknown field `parserOptions` at /clauses/0/forbid/ast/languageOptions",
  "python languageOptions.jsxPragma": "unknown field `jsxPragma` at /clauses/0/forbid/ast/languageOptions",
  "python languageOptions.jsxFragmentName": "unknown field `jsxFragmentName` at /clauses/0/forbid/ast/languageOptions",
  "python languageOptions.lib": "unknown field `lib` at /clauses/0/forbid/ast/languageOptions",
  "typescript languageOptions.pythonVersion": "unknown field `pythonVersion` at /clauses/0/forbid/ast/languageOptions",
  "typescript languageOptions.typingModules": "unknown field `typingModules` at /clauses/0/forbid/ast/languageOptions",
  "typescript languageOptions.extendGenerics": "unknown field `extendGenerics` at /clauses/0/forbid/ast/languageOptions",
  "javascript languageOptions.pythonVersion": "unknown field `pythonVersion` at /clauses/0/forbid/ast/languageOptions",
  "javascript languageOptions.typingModules": "unknown field `typingModules` at /clauses/0/forbid/ast/languageOptions",
  "javascript languageOptions.extendGenerics": "unknown field `extendGenerics` at /clauses/0/forbid/ast/languageOptions",
  "python when.ecmaVersion": "unknown field `ecmaVersion` at /clauses/0/forbid/ast/when",
  "python when.sourceType": "unknown field `sourceType` at /clauses/0/forbid/ast/when",
  "python when.ecmaFeatures": "unknown field `ecmaFeatures` at /clauses/0/forbid/ast/when",
  "typescript when.pythonVersion": "unknown field `pythonVersion` at /clauses/0/forbid/ast/when",
  "javascript when.pythonVersion": "unknown field `pythonVersion` at /clauses/0/forbid/ast/when",
  "php when.pythonVersion": "unknown field `pythonVersion` at /clauses/0/forbid/ast/when",
  "blade when.pythonVersion": "unknown field `pythonVersion` at /clauses/0/forbid/ast/when",
  "typescript file.module": "/clauses/0/forbid/ast/file/module: module paths are Python's; typescript has none",
  "typescript file.moduleName": "/clauses/0/forbid/ast/file/moduleName: module paths are Python's; typescript has none",
  "typescript file.package": "/clauses/0/forbid/ast/file/package: module paths are Python's; typescript has none",
  "typescript file.inPackage": "/clauses/0/forbid/ast/file/inPackage: module paths are Python's; typescript has none",
  "javascript file.module": "/clauses/0/forbid/ast/file/module: module paths are Python's; javascript has none",
  "javascript file.moduleName": "/clauses/0/forbid/ast/file/moduleName: module paths are Python's; javascript has none",
  "javascript file.package": "/clauses/0/forbid/ast/file/package: module paths are Python's; javascript has none",
  "javascript file.inPackage": "/clauses/0/forbid/ast/file/inPackage: module paths are Python's; javascript has none",
  "php file.module": "/clauses/0/forbid/ast/file/module: module paths are Python's; php has none",
  "php file.moduleName": "/clauses/0/forbid/ast/file/moduleName: module paths are Python's; php has none",
  "php file.package": "/clauses/0/forbid/ast/file/package: module paths are Python's; php has none",
  "php file.inPackage": "/clauses/0/forbid/ast/file/inPackage: module paths are Python's; php has none",
  "blade file.module": "/clauses/0/forbid/ast/file/module: module paths are Python's; blade has none",
  "blade file.moduleName": "/clauses/0/forbid/ast/file/moduleName: module paths are Python's; blade has none",
  "blade file.package": "/clauses/0/forbid/ast/file/package: module paths are Python's; blade has none",
  "blade file.inPackage": "/clauses/0/forbid/ast/file/inPackage: module paths are Python's; blade has none",
  "python file.declaration": "/clauses/0/forbid/ast/file/declaration: declaration files are TypeScript's; python has none",
  "python suppress.eslint": "/clauses/0/violation/suppress/eslint: eslint-disable comments are ESLint's; a python query has none",
  "python suppress.phpstan": "/clauses/0/violation/suppress/phpstan: @phpstan-ignore comments are PHPStan's; a python query has none",
  "python suppress.psalm": "/clauses/0/violation/suppress/psalm: @psalm-suppress tags are Psalm's; a python query has none",
  "python suppress.phpcs": "/clauses/0/violation/suppress/phpcs: phpcs: annotations are PHP_CodeSniffer's; a python query has none",
  "typescript suppress.noqa": "/clauses/0/violation/suppress/noqa: noqa comments are Ruff's; a typescript query has none",
  "typescript suppress.phpstan": "/clauses/0/violation/suppress/phpstan: @phpstan-ignore comments are PHPStan's; a typescript query has none",
  "typescript suppress.psalm": "/clauses/0/violation/suppress/psalm: @psalm-suppress tags are Psalm's; a typescript query has none",
  "typescript suppress.phpcs": "/clauses/0/violation/suppress/phpcs: phpcs: annotations are PHP_CodeSniffer's; a typescript query has none",
  "javascript suppress.noqa": "/clauses/0/violation/suppress/noqa: noqa comments are Ruff's; a typescript query has none",
  "javascript suppress.phpstan": "/clauses/0/violation/suppress/phpstan: @phpstan-ignore comments are PHPStan's; a typescript query has none",
  "javascript suppress.psalm": "/clauses/0/violation/suppress/psalm: @psalm-suppress tags are Psalm's; a typescript query has none",
  "javascript suppress.phpcs": "/clauses/0/violation/suppress/phpcs: phpcs: annotations are PHP_CodeSniffer's; a typescript query has none",
  "php suppress.eslint": "/clauses/0/violation/suppress/eslint: eslint-disable comments are ESLint's; a php query has none",
  "php suppress.noqa": "/clauses/0/violation/suppress/noqa: noqa comments are Ruff's; a php query has none",
  "blade suppress.eslint": "/clauses/0/violation/suppress/eslint: eslint-disable comments are ESLint's; a blade query has none",
  "blade suppress.noqa": "/clauses/0/violation/suppress/noqa: noqa comments are Ruff's; a blade query has none",
  "blade suppress.phpstan": "/clauses/0/violation/suppress/phpstan: @phpstan-ignore comments are PHPStan's; a blade query has none",
  "blade suppress.psalm": "/clauses/0/violation/suppress/psalm: @psalm-suppress tags are Psalm's; a blade query has none",
  "blade suppress.phpcs": "/clauses/0/violation/suppress/phpcs: phpcs: annotations are PHP_CodeSniffer's; a blade query has none",
  "typescript define utility formatArgs": "/clauses/0/define/utils/u/formatArgs: `formatArgs` is not available for typescript policies",
  "python define utility binds.builtin": "/clauses/0/define/utils/u/binds/builtin: configured globals are ESLint's; Python has none",
  "php query utility value.type str": "/clauses/0/forbid/ast/utils/u/value/type/0: `str` is not a PHP type name; expected one of int, float, string, bool, null, array",
  "python constraint binds.builtin": "/clauses/0/forbid/ast/constraints/A/binds/builtin: configured globals are ESLint's; Python has none",
};

describe("validateCodeRule refuses what a rule's language lacks, with the engine's reason", () => {
  it.each(cases)("%s", (name, rule) => {
    expect(validateCodeRule(rule)).toEqual({ ok: false, code: "invalid-rule", reason: ENGINE_REASONS[name] });
  });
});
