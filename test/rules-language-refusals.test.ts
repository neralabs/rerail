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

/** Where the engine refuses each case: the JSON pointer its reason names. */
const REFUSED_AT: Record<string, string> = {
  "typescript calls": "/clauses/0/forbid/ast/rule/calls",
  "typescript formatArgs": "/clauses/0/forbid/ast/rule/formatArgs",
  "javascript calls": "/clauses/0/forbid/ast/rule/calls",
  "javascript formatArgs": "/clauses/0/forbid/ast/rule/formatArgs",
  "python resolves.directives": "/clauses/0/forbid/ast/rule/resolves/directives",
  "python unresolved directives": "/clauses/0/forbid/ast/rule/resolves/directives",
  "typescript resolves.import": "/clauses/0/forbid/ast/rule/resolves/import",
  "typescript resolves.imported": "/clauses/0/forbid/ast/rule/resolves/imported",
  "typescript resolves.binding.latest": "/clauses/0/forbid/ast/rule/resolves/binding/latest",
  "typescript resolves.binding.outer": "/clauses/0/forbid/ast/rule/resolves/binding/outer",
  "typescript binds.latest": "/clauses/0/forbid/ast/rule/binds/latest",
  "typescript binds.outer": "/clauses/0/forbid/ast/rule/binds/outer",
  "typescript binds.redefines.sameImport": "/clauses/0/forbid/ast/rule/binds/redefines/sameImport",
  "javascript resolves.import": "/clauses/0/forbid/ast/rule/resolves/import",
  "javascript resolves.imported": "/clauses/0/forbid/ast/rule/resolves/imported",
  "javascript resolves.binding.latest": "/clauses/0/forbid/ast/rule/resolves/binding/latest",
  "javascript resolves.binding.outer": "/clauses/0/forbid/ast/rule/resolves/binding/outer",
  "javascript binds.latest": "/clauses/0/forbid/ast/rule/binds/latest",
  "javascript binds.outer": "/clauses/0/forbid/ast/rule/binds/outer",
  "javascript binds.redefines.sameImport": "/clauses/0/forbid/ast/rule/binds/redefines/sameImport",
  "python binds.redefines.every": "/clauses/0/forbid/ast/rule/binds/redefines/every",
  "php binds.redefines.sameImport": "/clauses/0/forbid/ast/rule/binds/redefines/sameImport",
  "blade binds.redefines.sameImport": "/clauses/0/forbid/ast/rule/binds/redefines/sameImport",
  "typescript resolves.beforeLocal": "/clauses/0/forbid/ast/rule/resolves/beforeLocal reads Python's binding order; TypeScript has none",
  "typescript resolves.unresolved star": "/clauses/0/forbid/ast/rule/resolves/unresolved",
  "typescript scriptDeclarations without languageOptions": "/clauses/0/forbid/ast/rule/resolves/scriptDeclarations",
  "typescript declared without languageOptions": "/clauses/0/forbid/ast/rule/resolves/declared",
  "typescript this without languageOptions": "/clauses/0/forbid/ast/rule/resolves/this",
  "typescript resolves.binding.stopAtGlobal": "/clauses/0/forbid/ast/rule/resolves/binding/stopAtGlobal",
  "typescript arguments signature /": "/clauses/0/forbid/ast/rule/arguments/signature",
  "typescript arguments signature *": "/clauses/0/forbid/ast/rule/arguments/signature",
  "typescript arguments mode syntactic": "/clauses/0/forbid/ast/rule/arguments/mode",
  "typescript compare structure": "/clauses/0/forbid/ast/rule/compare/text",
  "typescript compare structure-hash": "/clauses/0/forbid/ast/rule/compare/text",
  "typescript compare structure-parts": "/clauses/0/forbid/ast/rule/compare/text",
  "javascript resolves.beforeLocal": "/clauses/0/forbid/ast/rule/resolves/beforeLocal reads Python's binding order; TypeScript has none",
  "javascript resolves.unresolved star": "/clauses/0/forbid/ast/rule/resolves/unresolved",
  "javascript scriptDeclarations without languageOptions": "/clauses/0/forbid/ast/rule/resolves/scriptDeclarations",
  "javascript declared without languageOptions": "/clauses/0/forbid/ast/rule/resolves/declared",
  "javascript this without languageOptions": "/clauses/0/forbid/ast/rule/resolves/this",
  "javascript resolves.binding.stopAtGlobal": "/clauses/0/forbid/ast/rule/resolves/binding/stopAtGlobal",
  "javascript arguments signature /": "/clauses/0/forbid/ast/rule/arguments/signature",
  "javascript arguments signature *": "/clauses/0/forbid/ast/rule/arguments/signature",
  "javascript arguments mode syntactic": "/clauses/0/forbid/ast/rule/arguments/mode",
  "javascript compare structure": "/clauses/0/forbid/ast/rule/compare/text",
  "javascript compare structure-hash": "/clauses/0/forbid/ast/rule/compare/text",
  "javascript compare structure-parts": "/clauses/0/forbid/ast/rule/compare/text",
  "python resolves.this": "/clauses/0/forbid/ast/rule/resolves/this",
  "python resolves.scriptDeclarations": "/clauses/0/forbid/ast/rule/resolves/scriptDeclarations",
  "python declared without languageOptions": "/clauses/0/forbid/ast/rule/resolves/declared",
  "python resolves.binding.destructuredValue": "/clauses/0/forbid/ast/rule/resolves/binding/destructuredValue",
  "python arguments mode estree": "/clauses/0/forbid/ast/rule/arguments/mode",
  "python compare key": "/clauses/0/forbid/ast/rule/compare/text",
  "python compare member": "/clauses/0/forbid/ast/rule/compare/text",
  "python compare reference": "/clauses/0/forbid/ast/rule/compare/text",
  "python compare reference-no-static-keys": "/clauses/0/forbid/ast/rule/compare/text",
  "python compare name": "/clauses/0/forbid/ast/rule/compare/text",
  "php resolves.binding.stopAtGlobal false": "/clauses/0/forbid/ast/rule/resolves/binding/stopAtGlobal",
  "php arguments signature /": "/clauses/0/forbid/ast/rule/arguments/signature",
  "php arguments signature *": "/clauses/0/forbid/ast/rule/arguments/signature",
  "php arguments mode estree": "/clauses/0/forbid/ast/rule/arguments/mode",
  "php compare structure-parts": "/clauses/0/forbid/ast/rule/compare/text",
  "blade resolves.binding.stopAtGlobal false": "/clauses/0/forbid/ast/rule/resolves/binding/stopAtGlobal",
  "blade arguments signature /": "/clauses/0/forbid/ast/rule/arguments/signature",
  "blade arguments signature *": "/clauses/0/forbid/ast/rule/arguments/signature",
  "blade arguments mode estree": "/clauses/0/forbid/ast/rule/arguments/mode",
  "blade compare structure-parts": "/clauses/0/forbid/ast/rule/compare/text",
  "php resolves.this": "/clauses/0/forbid/ast/rule/resolves/this",
  "php resolves.sloppyFunctions": "/clauses/0/forbid/ast/rule/resolves/this",
  "php resolves.directives": "/clauses/0/forbid/ast/rule/resolves/directives",
  "php resolves.declared": "/clauses/0/forbid/ast/rule/resolves/declared",
  "php resolves.scriptDeclarations": "/clauses/0/forbid/ast/rule/resolves/scriptDeclarations",
  "php resolves.relative": "/clauses/0/forbid/ast/rule/resolves/relative",
  "php resolves.seenBefore": "/clauses/0/forbid/ast/rule/resolves/seenBefore",
  "php resolves.beforeLocal": "/clauses/0/forbid/ast/rule/resolves/beforeLocal",
  "php names beside variables": "/clauses/0/forbid/ast/rule/resolves",
  "php unresolved star": "/clauses/0/forbid/ast/rule/resolves/unresolved",
  "php global not a superglobal": "/clauses/0/forbid/ast/rule/resolves/global/1",
  "blade resolves.this": "/clauses/0/forbid/ast/rule/resolves/this",
  "blade resolves.sloppyFunctions": "/clauses/0/forbid/ast/rule/resolves/this",
  "blade resolves.directives": "/clauses/0/forbid/ast/rule/resolves/directives",
  "blade resolves.declared": "/clauses/0/forbid/ast/rule/resolves/declared",
  "blade resolves.scriptDeclarations": "/clauses/0/forbid/ast/rule/resolves/scriptDeclarations",
  "blade resolves.relative": "/clauses/0/forbid/ast/rule/resolves/relative",
  "blade resolves.seenBefore": "/clauses/0/forbid/ast/rule/resolves/seenBefore",
  "blade resolves.beforeLocal": "/clauses/0/forbid/ast/rule/resolves/beforeLocal",
  "blade names beside variables": "/clauses/0/forbid/ast/rule/resolves",
  "blade unresolved star": "/clauses/0/forbid/ast/rule/resolves/unresolved",
  "blade global not a superglobal": "/clauses/0/forbid/ast/rule/resolves/global/1",
  "python import of a PHP name": "/clauses/0/forbid/ast/rule/resolves/import/0",
  "python imported PHP name": "/clauses/0/forbid/ast/rule/resolves/imported/0",
  "php dotted import": "/clauses/0/forbid/ast/rule/resolves/import/0",
  "php dotted imported": "/clauses/0/forbid/ast/rule/resolves/imported/0",
  "blade dotted import": "/clauses/0/forbid/ast/rule/resolves/import/0",
  "blade dotted imported": "/clauses/0/forbid/ast/rule/resolves/imported/0",
  "python resolves.write": "/clauses/0/forbid/ast/rule/resolves/write",
  "php resolves.write": "/clauses/0/forbid/ast/rule/resolves/write",
  "blade resolves.write": "/clauses/0/forbid/ast/rule/resolves/write",
  "php binds.form import": "/clauses/0/forbid/ast/rule/binds/form",
  "php binds.scope module": "/clauses/0/forbid/ast/rule/binds/scope",
  "php binds.exported": "/clauses/0/forbid/ast/rule/binds/exported",
  "php binds.builtin": "/clauses/0/forbid/ast/rule/binds/builtin",
  "php binds.directives": "/clauses/0/forbid/ast/rule/binds/directives",
  "php resolves.binding.exported": "/clauses/0/forbid/ast/rule/resolves/binding/exported",
  "blade binds.form import": "/clauses/0/forbid/ast/rule/binds/form",
  "blade binds.scope module": "/clauses/0/forbid/ast/rule/binds/scope",
  "blade binds.exported": "/clauses/0/forbid/ast/rule/binds/exported",
  "blade binds.builtin": "/clauses/0/forbid/ast/rule/binds/builtin",
  "blade binds.directives": "/clauses/0/forbid/ast/rule/binds/directives",
  "blade resolves.binding.exported": "/clauses/0/forbid/ast/rule/resolves/binding/exported",
  "python binds.form static": "/clauses/0/forbid/ast/rule/binds/form",
  "typescript binds.form static": "/clauses/0/forbid/ast/rule/binds/form",
  "javascript binds.form static": "/clauses/0/forbid/ast/rule/binds/form",
  "python binds.scope block": "/clauses/0/forbid/ast/rule/binds/scope",
  "python binds.scope closure": "/clauses/0/forbid/ast/rule/binds/scope",
  "typescript binds.scope lambda": "/clauses/0/forbid/ast/rule/binds/scope",
  "typescript binds.scope closure": "/clauses/0/forbid/ast/rule/binds/scope",
  "javascript binds.scope lambda": "/clauses/0/forbid/ast/rule/binds/scope",
  "javascript binds.scope closure": "/clauses/0/forbid/ast/rule/binds/scope",
  "python binds.directives": "/clauses/0/forbid/ast/rule/binds/directives",
  "python binds.builtin": "/clauses/0/forbid/ast/rule/binds/builtin",
  "python binds.redefines.builtin": "/clauses/0/forbid/ast/rule/binds/redefines/builtin",
  "python resolves.binding.form static": "/clauses/0/forbid/ast/rule/resolves/binding/form",
  "python binds.occurrence": "/clauses/0/forbid/ast/rule/binds/occurrence",
  "python resolves.binding.occurrence": "/clauses/0/forbid/ast/rule/resolves/binding/occurrence",
  "python value.type string": "/clauses/0/forbid/ast/rule/value/type/0",
  "typescript value.type str": "/clauses/0/forbid/ast/rule/value/type/0",
  "javascript value.type str": "/clauses/0/forbid/ast/rule/value/type/0",
  "php value.type str": "/clauses/0/forbid/ast/rule/value/type/1",
  "blade value.type str": "/clauses/0/forbid/ast/rule/value/type/1",
  "typescript binding value type str": "/clauses/0/forbid/ast/rule/resolves/binding/value/value/type/0",
  "python parse eslintDirective": "/clauses/0/forbid/ast/rule/parse/grammar",
  "python parse printfFormat": "/clauses/0/forbid/ast/rule/parse/grammar",
  "python parse scanfFormat": "/clauses/0/forbid/ast/rule/parse/grammar",
  "python parse phpDoc": "/clauses/0/forbid/ast/rule/parse/grammar",
  "typescript parse percentFormat": "/clauses/0/forbid/ast/rule/parse/grammar",
  "typescript parse braceFormat": "/clauses/0/forbid/ast/rule/parse/grammar",
  "typescript parse annotation": "/clauses/0/forbid/ast/rule/parse/grammar",
  "typescript parse printfFormat": "/clauses/0/forbid/ast/rule/parse/grammar",
  "typescript parse scanfFormat": "/clauses/0/forbid/ast/rule/parse/grammar",
  "typescript parse phpDoc": "/clauses/0/forbid/ast/rule/parse/grammar",
  "javascript parse percentFormat": "/clauses/0/forbid/ast/rule/parse/grammar",
  "javascript parse braceFormat": "/clauses/0/forbid/ast/rule/parse/grammar",
  "javascript parse annotation": "/clauses/0/forbid/ast/rule/parse/grammar",
  "javascript parse printfFormat": "/clauses/0/forbid/ast/rule/parse/grammar",
  "javascript parse scanfFormat": "/clauses/0/forbid/ast/rule/parse/grammar",
  "javascript parse phpDoc": "/clauses/0/forbid/ast/rule/parse/grammar",
  "php parse percentFormat": "/clauses/0/forbid/ast/rule/parse/grammar",
  "php parse braceFormat": "/clauses/0/forbid/ast/rule/parse/grammar",
  "php parse annotation": "/clauses/0/forbid/ast/rule/parse/grammar",
  "php parse eslintDirective": "/clauses/0/forbid/ast/rule/parse/grammar",
  "blade parse percentFormat": "/clauses/0/forbid/ast/rule/parse/grammar",
  "blade parse braceFormat": "/clauses/0/forbid/ast/rule/parse/grammar",
  "blade parse annotation": "/clauses/0/forbid/ast/rule/parse/grammar",
  "blade parse eslintDirective": "/clauses/0/forbid/ast/rule/parse/grammar",
  "python style printf": "/clauses/0/forbid/ast/rule/formatArgs/style",
  "python style scanf": "/clauses/0/forbid/ast/rule/formatArgs/style",
  "php style percent": "/clauses/0/forbid/ast/rule/formatArgs/style",
  "php style brace": "/clauses/0/forbid/ast/rule/formatArgs/style",
  "blade style percent": "/clauses/0/forbid/ast/rule/formatArgs/style",
  "blade style brace": "/clauses/0/forbid/ast/rule/formatArgs/style",
  "php languageOptions": "/clauses/0/forbid/ast/languageOptions",
  "blade languageOptions": "/clauses/0/forbid/ast/languageOptions",
  "python languageOptions.ecmaVersion": "/clauses/0/forbid/ast/languageOptions",
  "python languageOptions.sourceType": "/clauses/0/forbid/ast/languageOptions",
  "python languageOptions.globals": "/clauses/0/forbid/ast/languageOptions",
  "python languageOptions.parserOptions": "/clauses/0/forbid/ast/languageOptions",
  "python languageOptions.jsxPragma": "/clauses/0/forbid/ast/languageOptions",
  "python languageOptions.jsxFragmentName": "/clauses/0/forbid/ast/languageOptions",
  "python languageOptions.lib": "/clauses/0/forbid/ast/languageOptions",
  "typescript languageOptions.pythonVersion": "/clauses/0/forbid/ast/languageOptions",
  "typescript languageOptions.typingModules": "/clauses/0/forbid/ast/languageOptions",
  "typescript languageOptions.extendGenerics": "/clauses/0/forbid/ast/languageOptions",
  "javascript languageOptions.pythonVersion": "/clauses/0/forbid/ast/languageOptions",
  "javascript languageOptions.typingModules": "/clauses/0/forbid/ast/languageOptions",
  "javascript languageOptions.extendGenerics": "/clauses/0/forbid/ast/languageOptions",
  "python when.ecmaVersion": "/clauses/0/forbid/ast/when",
  "python when.sourceType": "/clauses/0/forbid/ast/when",
  "python when.ecmaFeatures": "/clauses/0/forbid/ast/when",
  "typescript when.pythonVersion": "/clauses/0/forbid/ast/when",
  "javascript when.pythonVersion": "/clauses/0/forbid/ast/when",
  "php when.pythonVersion": "/clauses/0/forbid/ast/when",
  "blade when.pythonVersion": "/clauses/0/forbid/ast/when",
  "typescript file.module": "/clauses/0/forbid/ast/file/module",
  "typescript file.moduleName": "/clauses/0/forbid/ast/file/moduleName",
  "typescript file.package": "/clauses/0/forbid/ast/file/package",
  "typescript file.inPackage": "/clauses/0/forbid/ast/file/inPackage",
  "javascript file.module": "/clauses/0/forbid/ast/file/module",
  "javascript file.moduleName": "/clauses/0/forbid/ast/file/moduleName",
  "javascript file.package": "/clauses/0/forbid/ast/file/package",
  "javascript file.inPackage": "/clauses/0/forbid/ast/file/inPackage",
  "php file.module": "/clauses/0/forbid/ast/file/module",
  "php file.moduleName": "/clauses/0/forbid/ast/file/moduleName",
  "php file.package": "/clauses/0/forbid/ast/file/package",
  "php file.inPackage": "/clauses/0/forbid/ast/file/inPackage",
  "blade file.module": "/clauses/0/forbid/ast/file/module",
  "blade file.moduleName": "/clauses/0/forbid/ast/file/moduleName",
  "blade file.package": "/clauses/0/forbid/ast/file/package",
  "blade file.inPackage": "/clauses/0/forbid/ast/file/inPackage",
  "python file.declaration": "/clauses/0/forbid/ast/file/declaration",
  "python suppress.eslint": "/clauses/0/violation/suppress/eslint",
  "python suppress.phpstan": "/clauses/0/violation/suppress/phpstan",
  "python suppress.psalm": "/clauses/0/violation/suppress/psalm",
  "python suppress.phpcs": "/clauses/0/violation/suppress/phpcs",
  "typescript suppress.noqa": "/clauses/0/violation/suppress/noqa",
  "typescript suppress.phpstan": "/clauses/0/violation/suppress/phpstan",
  "typescript suppress.psalm": "/clauses/0/violation/suppress/psalm",
  "typescript suppress.phpcs": "/clauses/0/violation/suppress/phpcs",
  "javascript suppress.noqa": "/clauses/0/violation/suppress/noqa",
  "javascript suppress.phpstan": "/clauses/0/violation/suppress/phpstan",
  "javascript suppress.psalm": "/clauses/0/violation/suppress/psalm",
  "javascript suppress.phpcs": "/clauses/0/violation/suppress/phpcs",
  "php suppress.eslint": "/clauses/0/violation/suppress/eslint",
  "php suppress.noqa": "/clauses/0/violation/suppress/noqa",
  "blade suppress.eslint": "/clauses/0/violation/suppress/eslint",
  "blade suppress.noqa": "/clauses/0/violation/suppress/noqa",
  "blade suppress.phpstan": "/clauses/0/violation/suppress/phpstan",
  "blade suppress.psalm": "/clauses/0/violation/suppress/psalm",
  "blade suppress.phpcs": "/clauses/0/violation/suppress/phpcs",
  "typescript define utility formatArgs": "/clauses/0/define/utils/u/formatArgs",
  "python define utility binds.builtin": "/clauses/0/define/utils/u/binds/builtin",
  "php query utility value.type str": "/clauses/0/forbid/ast/utils/u/value/type/0",
  "python constraint binds.builtin": "/clauses/0/forbid/ast/constraints/A/binds/builtin",
};

describe("validateCodeRule refuses what a rule's language lacks, where the rule uses it", () => {
  it.each(cases)("%s", (name, rule) => {
    const validation = validateCodeRule(rule);
    expect(validation).toMatchObject({ ok: false, code: "invalid-rule" });
    expect(validation.ok ? undefined : validation.reason).toContain(REFUSED_AT[name]);
  });
});
