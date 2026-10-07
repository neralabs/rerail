/**
 * Rule × source cases that pin Rerail's relational semantics to upstream ast-grep 0.45.3. Each case runs through the
 * ast-grep CLI and through Rerail's kernel (stateless and anchor-indexed workspace), and the match sets must agree.
 * The CLI rejects a top-level rule without a node kind (e.g. regex-only), so every rule names a kind or pattern.
 */
export interface UpstreamCase {
  id: string;
  language: "typescript" | "python";
  source: string;
  rule: Record<string, unknown>;
  /** Local utility rules `rule` and `constraints` may reference with `matches`. */
  utils?: Record<string, Record<string, unknown>>;
  constraints?: Record<string, Record<string, unknown>>;
  /** Rerail's ast `languageOptions`; upstream ignores the field beside `rule` in a rule config. */
  languageOptions?: Record<string, unknown>;
}

const blocks = "function a() { x(); }\nfunction b() { y(); }\n";

export const UPSTREAM_RELATIONAL_CASES: UpstreamCase[] = [
  // precedes/follows: siblings only.
  {
    id: "follows-end-stays-in-parent",
    language: "typescript",
    source: blocks,
    rule: { kind: "expression_statement", has: { pattern: "y()" }, follows: { stopBy: "end", kind: "expression_statement" } },
  },
  {
    id: "precedes-end-stays-in-parent",
    language: "typescript",
    source: blocks,
    rule: { kind: "expression_statement", has: { pattern: "x()" }, precedes: { stopBy: "end", kind: "expression_statement" } },
  },
  {
    id: "precedes-end-same-level",
    language: "typescript",
    source: "a();\nif (c) { b(); }\nd();\n",
    rule: { kind: "expression_statement", precedes: { stopBy: "end", kind: "expression_statement" } },
  },
  {
    id: "follows-end-same-level",
    language: "typescript",
    source: "a();\nif (c) { b(); }\nd();\n",
    rule: { kind: "expression_statement", follows: { stopBy: "end", kind: "if_statement" } },
  },
  {
    id: "neighbor-is-the-sibling-not-its-subtree",
    language: "typescript",
    source: "a();\nif (c) { b(); }\n",
    rule: { kind: "expression_statement", precedes: { kind: "expression_statement" } },
  },
  {
    id: "neighbor-does-not-climb",
    language: "typescript",
    source: "if (c) a();\nb();\n",
    rule: { kind: "expression_statement", has: { pattern: "a()" }, precedes: { kind: "expression_statement" } },
  },
  {
    id: "follows-neighbor-does-not-climb",
    language: "typescript",
    source: "x = 1;\na() || b;\n",
    rule: { pattern: "a()", follows: { kind: "expression_statement" } },
  },
  {
    id: "comment-is-a-sibling",
    language: "typescript",
    source: "a(); // note\nb();\nc();\n",
    rule: { kind: "expression_statement", precedes: { kind: "expression_statement" } },
  },
  {
    id: "end-relation-under-not",
    language: "typescript",
    source: "function f() { return 1; }\nfunction g() { if (a) {} return 2; }\n",
    rule: { kind: "return_statement", not: { follows: { stopBy: "end", kind: "if_statement" } } },
  },
  {
    id: "end-relation-under-not-layout-independent",
    language: "typescript",
    source: "function g() { if (a) {} return 2; }\nfunction f() { return 1; }\n",
    rule: { kind: "return_statement", not: { follows: { stopBy: "end", kind: "if_statement" } } },
  },
  {
    id: "root-precedes-end-walks-its-children",
    language: "typescript",
    source: "a;\nb;\n",
    rule: { kind: "program", precedes: { stopBy: "end", kind: "expression_statement" } },
  },
  {
    id: "root-precedes-end-single-child",
    language: "typescript",
    source: "a;\n",
    rule: { kind: "program", precedes: { stopBy: "end", kind: "expression_statement" } },
  },
  {
    id: "python-call-has-no-statement-siblings",
    language: "python",
    source: "def run():\n  prepare()\n  execute()\n",
    rule: { pattern: "prepare()", precedes: { stopBy: "end", pattern: "execute()" } },
  },
  {
    id: "python-statement-siblings",
    language: "python",
    source: "def run():\n  prepare()\n  unrelated()\n  execute()\n",
    rule: { kind: "expression_statement", has: { pattern: "execute()" }, follows: { stopBy: "end", has: { pattern: "prepare()" } } },
  },
  // Anonymous tokens are children and siblings; only regex (or a pattern of the token) names them.
  {
    id: "comma-is-the-neighbor",
    language: "typescript",
    source: "[1, 2];\n",
    rule: { kind: "number", follows: { regex: "^,$" } },
  },
  {
    id: "paren-precedes-first-argument",
    language: "typescript",
    source: "f(a, b);\n",
    rule: { kind: "identifier", follows: { regex: String.raw`^\($` } },
  },
  {
    id: "has-anonymous-child",
    language: "typescript",
    source: "f(a, b);\ng(c);\n",
    rule: { kind: "arguments", has: { regex: "^,$" } },
  },
  {
    id: "has-async-keyword",
    language: "typescript",
    source: "const f = async () => 1;\nconst g = () => 2;\n",
    rule: { kind: "arrow_function", has: { regex: "^async$" } },
  },
  {
    id: "inside-neighbor",
    language: "typescript",
    source: "f(a, g(b));\n",
    rule: { kind: "identifier", inside: { kind: "arguments" } },
  },
  // One metavariable environment threads through relations.
  {
    id: "has-shares-bindings",
    language: "typescript",
    source: "wrap(x, g(x));\nwrap(y, g(z));\n",
    rule: { pattern: "wrap($A, $B)", has: { stopBy: "end", pattern: "g($A)" } },
  },
  {
    id: "not-reads-bindings",
    language: "typescript",
    source: "wrap(x, g(x));\nwrap(y, g(z));\n",
    rule: { pattern: "wrap($A, $B)", not: { has: { stopBy: "end", pattern: "g($A)" } } },
  },
  {
    id: "inside-shares-bindings",
    language: "typescript",
    source: "function a() { use(a); use(b); }\n",
    rule: { pattern: "use($A)", inside: { stopBy: "end", kind: "function_declaration", has: { kind: "identifier", pattern: "$A" } } },
  },
  {
    id: "precedes-shares-bindings",
    language: "typescript",
    source: "function f() { open(x); close(y); }\nfunction g() { open(z); other(); close(z); }\n",
    rule: { kind: "expression_statement", has: { pattern: "open($A)" }, precedes: { stopBy: "end", has: { pattern: "close($A)" } } },
  },
  {
    id: "first-related-node-binds",
    language: "typescript",
    source: "function p() { f(1); f(2); g(2); }\nfunction q() { f(2); f(1); g(2); }\n",
    rule: { kind: "statement_block", all: [{ has: { stopBy: "end", pattern: "f($A)" } }, { has: { stopBy: "end", pattern: "g($A)" } }] },
  },
  {
    id: "composite-terms-bind-before-relations",
    language: "typescript",
    source: "function p() { f(1); f(2); g(2); }\n",
    rule: { kind: "statement_block", has: { stopBy: "end", pattern: "f($A)" }, all: [{ has: { stopBy: "end", pattern: "g($A)" } }] },
  },
  {
    id: "any-branches-start-clean",
    language: "typescript",
    source: "f(1, g(1));\nf(1, g(2));\nh(g(3));\n",
    rule: { kind: "call_expression", has: { stopBy: "end", pattern: "g($A)" }, any: [{ pattern: "f($A, $$$)" }, { pattern: "h($A)" }] },
  },
  {
    id: "relation-captures-are-reported",
    language: "typescript",
    source: "outer(inner(1), 2);\n",
    rule: { pattern: "outer($$$)", has: { stopBy: "end", pattern: "inner($X)" } },
  },
  // Contextual patterns: the selector node alone is matched; the context only guides parsing.
  {
    id: "contextual-selector-ignores-context",
    language: "typescript",
    source: "var a = c, b;\nvar x = 1;\n",
    rule: { pattern: { context: "var $A = $B, $B", selector: "variable_declarator" } },
  },
  {
    id: "contextual-switch-case",
    language: "typescript",
    source: "switch (x) { case 1: case 2: break; }\n",
    rule: { pattern: { context: "switch ($X) { $$$A case $V: case $V: $$$B }", selector: "switch_case" } },
  },
];
