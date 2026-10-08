import type { UpstreamCase } from "./upstream-relational-cases.js";

/**
 * Cases that pin `field` on `inside` and `has` to upstream ast-grep 0.45.3 (`rule/relational_rule.rs`): `inside` tests
 * only the ancestors its walk enters through the field; `has` starts at the node's child in the field, tests it alone
 * (`neighbor`), with its descendants (`end`), or, with a stop rule, it and then its children only (upstream's one
 * level). Each case runs through the CLI and both Rerail paths.
 */
const functions = "function f() {}\nfunction g() { a(); }\nfunction h() { { b(); } }\nfunction k() { return c(); }\n";
const conditions = "if (a && x) { x; }\nif (x) { y(); }\nwhile (x) { x; }\n";
const python = "def f(x=(yield)):\n    yield 1\n    def g():\n        return (yield 2)\n\nclass C:\n    yield 3\n\nyield 4\nlambda: (yield 5)\n";

export const UPSTREAM_FIELD_CASES: UpstreamCase[] = [
  {
    id: "inside-field-neighbor",
    language: "typescript",
    source: "const a = b;\nlet c = d, e = f;\n",
    rule: { kind: "identifier", inside: { kind: "variable_declarator", field: "name" } },
  },
  {
    id: "inside-field-end",
    language: "typescript",
    source: conditions,
    rule: { kind: "identifier", regex: "^x$", inside: { kind: "if_statement", field: "condition", stopBy: "end" } },
  },
  {
    id: "not-inside-field-end",
    language: "typescript",
    source: conditions,
    rule: { kind: "identifier", regex: "^x$", not: { inside: { kind: "if_statement", field: "condition", stopBy: "end" } } },
  },
  {
    id: "inside-field-stop-rule",
    language: "typescript",
    source: conditions,
    rule: { kind: "identifier", regex: "^x$", inside: { kind: "if_statement", field: "condition", stopBy: { kind: "if_statement" } } },
  },
  {
    id: "inside-field-stop-rule-before",
    language: "typescript",
    source: conditions,
    rule: { kind: "identifier", regex: "^x$", inside: { kind: "if_statement", field: "condition", stopBy: { kind: "binary_expression" } } },
  },
  {
    id: "inside-field-captures",
    language: "typescript",
    source: "const a = f(b);\nconst c = g(d, e);\n",
    rule: { kind: "identifier", inside: { pattern: "$F($$$)", field: "arguments", stopBy: "end" } },
  },
  {
    id: "has-field-neighbor",
    language: "typescript",
    source: functions,
    rule: { kind: "function_declaration", has: { field: "body", regex: "^\\{\\s*\\}$" } },
  },
  {
    id: "has-field-end",
    language: "typescript",
    source: functions,
    rule: { kind: "function_declaration", has: { field: "body", stopBy: "end", kind: "call_expression", pattern: "$C()" } },
  },
  {
    id: "has-field-end-includes-the-field-child",
    language: "typescript",
    source: "a(b);\nc.d(e);\n",
    rule: { kind: "call_expression", has: { field: "function", stopBy: "end", kind: "identifier" } },
  },
  {
    id: "has-field-stop-rule-one-level",
    language: "typescript",
    source: functions,
    rule: { kind: "function_declaration", has: { field: "body", stopBy: { kind: "return_statement" }, kind: "expression_statement" } },
  },
  {
    id: "has-field-stop-rule-at-the-field-child",
    language: "typescript",
    source: functions,
    rule: { kind: "function_declaration", not: { has: { field: "body", stopBy: { kind: "statement_block" }, kind: "expression_statement" } } },
  },
  {
    id: "has-field-stop-rule-descends-no-further",
    language: "typescript",
    source: functions,
    rule: { kind: "function_declaration", not: { has: { field: "body", stopBy: { kind: "return_statement" }, kind: "call_expression" } } },
  },
  {
    id: "has-field-missing",
    language: "typescript",
    source: "const k = x => x;\nconst l = (y) => y;\n",
    rule: { kind: "arrow_function", has: { field: "parameters", kind: "formal_parameters" } },
  },
  {
    id: "python-yield-outside-function-body",
    language: "python",
    source: python,
    rule: { kind: "yield", not: { inside: { any: [{ kind: "function_definition" }, { kind: "lambda" }], field: "body", stopBy: "end" } } },
  },
  {
    id: "python-has-field-in-a-utility",
    language: "python",
    source: "if a:\n    pass\nelif b:\n    x = 1\nif c:\n    y = 2\n",
    rule: { kind: "if_statement", matches: "empty-body" },
    utils: { "empty-body": { has: { field: "consequence", kind: "block", has: { kind: "pass_statement" } } } },
  },
];
