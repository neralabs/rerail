import type { UpstreamCase } from "./upstream-relational-cases.js";

/**
 * Cases that pin Rerail's `utils`/`matches` and `constraints` to upstream ast-grep 0.45.3: local utilities share the
 * caller's metavariable environment and may recurse through relations; constraints filter single-node captures after
 * the rule matches, with the same environment. Each runs through the CLI and both Rerail paths like the relational cases.
 */
export const UPSTREAM_UTILS_CASES: UpstreamCase[] = [
  {
    id: "util-recursion-through-has",
    language: "typescript",
    source: "async function f() { return ((((await x)))); }\nasync function g() { return (x); }\nasync function h() { return await y; }\n",
    rule: { kind: "return_statement", has: { matches: "tail-await" } },
    utils: {
      "tail-await": { any: [{ kind: "await_expression" }, { kind: "parenthesized_expression", has: { matches: "tail-await" } }] },
    },
  },
  {
    id: "util-recursion-through-inside",
    language: "typescript",
    source: "function f() { if (a) { while (b) { g(); } } }\nh();\n",
    rule: { pattern: "$F()", matches: "under-function" },
    utils: { "under-function": { inside: { any: [{ kind: "function_declaration" }, { matches: "under-function" }] } } },
  },
  {
    id: "util-recursion-through-precedes",
    language: "typescript",
    source: "a();\nb();\nc();\nlast;\nd();\n",
    rule: { kind: "expression_statement", matches: "calls-until-last" },
    utils: {
      "calls-until-last": {
        has: { kind: "call_expression" },
        precedes: { any: [{ pattern: "last;" }, { matches: "calls-until-last" }] },
      },
    },
  },
  {
    id: "util-shares-rule-bindings",
    language: "typescript",
    source: "g(g);\ng(h);\n",
    rule: { pattern: "$F($A)", matches: "calls-itself" },
    utils: { "calls-itself": { pattern: "$F($F)" } },
  },
  {
    id: "util-bindings-reach-the-rule",
    language: "typescript",
    source: "wrap(x, g(x));\nwrap(y, g(z));\n",
    rule: { kind: "call_expression", all: [{ matches: "wrap-call" }, { has: { stopBy: "end", pattern: "g($A)" } }] },
    utils: { "wrap-call": { pattern: "wrap($A, $$$)" } },
  },
  {
    id: "util-under-not",
    language: "typescript",
    source: "function f() { a(); }\nb();\n",
    rule: { pattern: "$F()", not: { matches: "in-function" } },
    utils: { "in-function": { inside: { stopBy: "end", kind: "function_declaration" } } },
  },
  {
    id: "util-uses-util",
    language: "typescript",
    source: "useState(1);\nuser(2);\nx.useMemo(3);\n",
    rule: { kind: "call_expression", matches: "hook-call" },
    utils: {
      "hook-call": { has: { kind: "identifier", matches: "hook-name" } },
      "hook-name": { regex: "^use[A-Z]" },
    },
  },
  {
    id: "util-relation-binds-first-node",
    language: "typescript",
    source: "function p() { f(1); f(2); g(2); }\nfunction q() { f(2); f(1); g(2); }\n",
    rule: { kind: "statement_block", all: [{ matches: "has-f" }, { has: { stopBy: "end", pattern: "g($A)" } }] },
    utils: { "has-f": { has: { stopBy: "end", pattern: "f($A)" } } },
  },
  {
    id: "python-util-recursion",
    language: "python",
    source: "def f():\n    return (((await_it(x))))\n\ndef g():\n    return x\n",
    rule: { kind: "return_statement", has: { matches: "wrapped-call" } },
    utils: { "wrapped-call": { any: [{ kind: "call" }, { kind: "parenthesized_expression", has: { matches: "wrapped-call" } }] } },
  },
  // constraints
  {
    id: "constraint-regex-on-capture",
    language: "typescript",
    source: "useState(1);\nuser(2);\nuseEffect(f);\n",
    rule: { pattern: "$FN($$$ARGS)" },
    constraints: { FN: { regex: "^use[A-Z]" } },
  },
  {
    id: "constraint-on-relation-capture",
    language: "typescript",
    source: "if (ready) { useState(1); }\nif (a.b) { useMemo(g); }\nif (ok) { log(); }\n",
    rule: { pattern: "$HOOK($$$)", inside: { stopBy: "end", kind: "if_statement", has: { pattern: "($COND)" } } },
    constraints: { HOOK: { regex: "^use[A-Z]" }, COND: { kind: "identifier" } },
  },
  {
    id: "constraint-with-not",
    language: "typescript",
    source: "a + 1;\na + b;\n-c;\n",
    rule: { any: [{ pattern: "$X + $Y" }, { pattern: "-$X" }] },
    constraints: { Y: { not: { kind: "number" } } },
  },
  {
    id: "constraint-does-not-backtrack",
    language: "typescript",
    source: "function p() { f(1); f(x); }\nfunction q() { f(x); f(1); }\n",
    rule: { kind: "statement_block", has: { stopBy: "end", pattern: "f($A)" } },
    constraints: { A: { kind: "identifier" } },
  },
  {
    id: "constraint-binds-into-the-witness",
    language: "typescript",
    source: "f(a.b);\nf(c);\n",
    rule: { pattern: "f($ARG)" },
    constraints: { ARG: { pattern: "$OBJ.$PROP" } },
  },
  {
    id: "constraint-reads-rule-bindings",
    language: "typescript",
    source: "set(a, a.x);\nset(a, b.x);\n",
    rule: { pattern: "set($T, $V)" },
    constraints: { V: { pattern: "$T.$P" } },
  },
  {
    id: "constraint-uses-util",
    language: "typescript",
    source: "useState(1);\nuser(2);\n",
    rule: { pattern: "$FN($$$)" },
    utils: { "hook-name": { regex: "^use[A-Z]" } },
    constraints: { FN: { matches: "hook-name" } },
  },
  {
    id: "constraint-on-unbound-branch",
    language: "typescript",
    source: "g(1);\nh(2);\n",
    rule: { any: [{ pattern: "g($A)" }, { pattern: "h($B)" }] },
    constraints: { A: { kind: "identifier" } },
  },
  {
    id: "hook-called-in-conditional",
    language: "typescript",
    source: "function C() {\n  if (x) { useEffect(f); }\n  const [s] = useState(0);\n  return x ? useMemo(g) : null;\n}\n",
    rule: {
      pattern: "$HOOK($$$)",
      inside: { stopBy: "end", any: [{ kind: "if_statement" }, { kind: "ternary_expression" }, { kind: "for_statement" }] },
    },
    constraints: { HOOK: { regex: "^use[A-Z]" } },
  },
];
