// Builds the TypeScript pack's @vitest/eslint-plugin 1.6.27 candidates: packs/typescript/candidates/rules/vitest-*.json.
// The JSON files are the committed artifacts; this script is their source. It spells once the test-function check the
// three ports share and writes each document the utilities it needs. Run offline by a maintainer, never in CI; the
// output is deterministic.
//
// Usage (from the repository root): node packs/typescript/candidates/build-vitest-policies.mjs
//
// Upstream reports a call only when parseVitestFnCall classifies it, and that reads the chain's head identifier
// through scope (resolveVitestFn and resolveScope in src/utils/parse-vitest-fn-call.ts). The head is Vitest's when:
// - it has no binding (a global);
// - its binding is a named value import from `vitest`, or a key destructured from `require('vitest')` or
//   `await import('vitest')`; the function is then the imported name (`import { test as it }` is `test`);
// - it is declared `const x = <root>.extend(...)` whose root is such a binding of a test function (`it`, `test`,
//   `fit`, `xit`, `xtest`, `bench`), or another such `.extend`; the function is the root's;
// - it is a test context: a parameter of a function passed to a call of a test function's name that also takes a
//   plain identifier parameter, or the first property of a function's first object-pattern parameter.
// Any other binding (`node:test`, `bun:test`, AVA, a local, a default or namespace import) is not Vitest's. Each port
// asks which functions it reports: `no-focused-tests` only `describe`, `it` and `test`, for instance.
import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const RULES = join(dirname(fileURLToPath(import.meta.url)), "rules");

const SCOPE = {
  include: ["**/*.test.ts", "**/*.test.tsx", "**/*.spec.ts", "**/*.spec.tsx", "**/*.test.mts", "**/*.spec.mts", "**/__tests__/**/*.ts", "**/__tests__/**/*.tsx"],
  exclude: [
    "**/node_modules/**",
    "**/dist/**",
    "**/build/**",
    "**/out/**",
    "**/.next/**",
    "**/coverage/**",
    "**/vendor/**",
    "**/.venv/**",
    "**/.yarn/**",
    "**/generated/**",
    "**/__generated__/**",
    "**/*.generated.*",
    "**/*.gen.*",
    "**/*.min.js",
  ],
};

/** Upstream's TestCaseName: the functions whose calls are tests, and the only roots `.extend` follows. */
const TEST_CASES = ["fit", "it", "test", "xit", "xtest", "bench"];
/**
 * The function names each port reports, as upstream classifies them. `focusable`: no-focused-tests reports `.only`
 * on `describe`, `it` and `test` alone. `skippable`: the chains with a `.skip` member (valid-vitest-fn-call-chains.ts),
 * and `expect`, `expectTypeOf`, `vi` and `vitest`, whose chains upstream does not check (an `it` destructured as
 * `{ expect: it }` reports `it.skip()`). `xdisabled`: the `x` functions. `test`: TestCaseName (expect-expect and a
 * test without a function). `block`: DescribeAlias and TestCaseName, the calls whose `{ skip: true }` option counts.
 */
const FUNCTIONS = {
  focusable: ["describe", "it", "test"],
  skippable: ["bench", "describe", "it", "suite", "test", "expect", "expectTypeOf", "vi", "vitest"],
  xdisabled: ["xdescribe", "xit", "xtest"],
  test: TEST_CASES,
  block: ["describe", "fdescribe", "xdescribe", ...TEST_CASES],
};

const oneOf = names => `^(?:${names.join("|")})$`;
/**
 * A property key upstream's `isSupportedAccessor` reads as one of `names` (`getAccessorValue`): an identifier, a string
 * literal by its value (escapes decoded), a template literal by its raw text (upstream reads `quasis[0].value.raw`), or
 * the same inside computed brackets (`["it"]`, `[it]`).
 */
const accessorKey = names => {
  const literal = {
    any: [
      { kind: "string", value: { regex: oneOf(names) } },
      { kind: "template_string", regex: `^\`(?:${names.join("|")})\`$` },
    ],
  };
  return {
    any: [
      { kind: "property_identifier", regex: oneOf(names) },
      literal,
      { kind: "computed_property_name", has: { any: [{ kind: "identifier", regex: oneOf(names) }, literal] } },
    ],
  };
};

/**
 * Upstream's ValidVitestFnCallChains (valid-vitest-fn-call-chains.ts), as the grammar that spells it exactly: after the
 * head, an optional `skipIf`/`runIf`, then distinct modifiers, then an optional `each`/`for`, three links at most; `it`
 * and `test` also take `extend` and `scoped` alone; `x` functions take `each` alone; `fit` and `fdescribe` take
 * nothing. A call whose chain is not one of these is no Vitest call upstream.
 */
const CHAIN_GRAMMAR = {
  test: { heads: ["it", "test"], modifiers: ["skip", "only", "concurrent", "sequential", "todo", "fails"], conditions: ["skipIf", "runIf"], terminals: ["each", "for"], alone: ["extend", "scoped"] },
  describe: { heads: ["describe", "suite"], modifiers: ["skip", "only", "concurrent", "sequential", "shuffle", "todo"], conditions: ["skipIf", "runIf"], terminals: ["each", "for"], alone: [] },
  bench: { heads: ["bench"], modifiers: ["skip", "only", "todo"], conditions: ["skipIf", "runIf"], terminals: [], alone: [] },
  x: { heads: ["xit", "xtest", "xdescribe"], modifiers: [], conditions: [], terminals: [], alone: ["each"] },
  f: { heads: ["fit", "fdescribe"], modifiers: [], conditions: [], terminals: [], alone: [] },
};

/** The member chains (links after the head) one grammar entry allows. */
function chainLinks({ modifiers, conditions, terminals, alone }) {
  const chains = [[], ...alone.map(link => [link])];
  const permutations = (pool, length) =>
    length === 0 ? [[]] : pool.flatMap(link => permutations(pool.filter(other => other !== link), length - 1).map(rest => [link, ...rest]));
  for (const condition of [[], ...conditions.map(link => [link])]) {
    for (let count = 0; count <= modifiers.length; count += 1) {
      for (const middle of permutations(modifiers, count)) {
        for (const terminal of [[], ...terminals.map(link => [link])]) {
          const links = [...condition, ...middle, ...terminal];
          if (links.length > 0 && links.length <= 3) chains.push(links);
        }
      }
    }
  }
  return chains;
}

/** An alternation matching exactly the given link lists as `.a.b` text, factored by common prefixes. */
function linksPattern(chains) {
  const tree = new Map();
  let end = false;
  for (const [first, ...rest] of chains) {
    if (first === undefined) end = true;
    else tree.set(first, [...(tree.get(first) ?? []), rest]);
  }
  const branches = [...tree].sort(([a], [b]) => (a < b ? -1 : 1)).map(([link, rests]) => `\\.${link}${linksPattern(rests)}`);
  if (branches.length === 0) return "";
  return `(?:${branches.join("|")})${end ? "?" : ""}`;
}

/**
 * A regex over a callee's text: one of `heads` followed by a valid chain for it whose links pass `keep` (for instance,
 * those that include `.only`, the way upstream finds `.only` among a valid chain's members).
 */
function validChain(heads, keep = () => true) {
  const alternatives = [];
  for (const grammar of Object.values(CHAIN_GRAMMAR)) {
    const names = grammar.heads.filter(head => heads.includes(head));
    if (names.length === 0) continue;
    const chains = chainLinks(grammar).filter(keep);
    if (chains.length > 0) alternatives.push(`(?:${names.join("|")})${linksPattern(chains)}`);
  }
  return `^(?:${alternatives.join("|")})$`;
}
const including = member => links => links.includes(member);
/** The modifiers the expect-expect port reads on a test (upstream skips `.skip`, `.todo`, `.extend` and `.scoped`). */
const EXPECT_LINKS = ["only", "concurrent", "sequential", "fails"];
const FUNCTION_KINDS = [{ kind: "arrow_function" }, { kind: "function_expression" }, { kind: "generator_function" }];
const PARAMETER = { any: [{ kind: "required_parameter" }, { kind: "optional_parameter" }] };

/**
 * Module-source utilities, read as upstream's ESTree resolver reads them: the argument that names the module is a string
 * literal whose value is `vitest` (escapes decoded) or a template literal whose raw text is (`getStringValue`), comments
 * and trailing commas aside, and parentheses around an expression are transparent (ESTree keeps none).
 */
const MODULE_SOURCE = {
  "vitest-source": { any: [{ kind: "string", value: { eq: "vitest" } }, { kind: "template_string", regex: "^`vitest`$" }] },
  "vitest-module-argument": { kind: "arguments", has: { nthChild: { position: 1, ofRule: { not: { kind: "comment" } } }, matches: "vitest-source" } },
  // `require('vitest')` or `await import('vitest')` (findImportSourceNode); `await require(...)` is neither.
  "vitest-module": {
    any: [
      {
        kind: "call_expression",
        all: [{ has: { field: "function", kind: "identifier", regex: "^require$" } }, { has: { field: "arguments", matches: "vitest-module-argument" } }],
      },
      { kind: "await_expression", has: { matches: "vitest-dynamic-import" } },
      { kind: "parenthesized_expression", has: { matches: "vitest-module" } },
    ],
  },
  "vitest-dynamic-import": {
    any: [
      { kind: "call_expression", all: [{ has: { field: "function", kind: "import" } }, { has: { field: "arguments", matches: "vitest-module-argument" } }] },
      { kind: "parenthesized_expression", has: { matches: "vitest-dynamic-import" } },
    ],
  },
};

/**
 * An object pattern in the name of a declarator whose value is `require('vitest')` or `await import('vitest')`, at any
 * depth (`{ a: { it } }`), as upstream reads the declarator's initializer whatever the pattern.
 */
const VITEST_DESTRUCTURING = {
  kind: "object_pattern",
  inside: { kind: "variable_declarator", stopBy: "end", has: { field: "value", matches: "vitest-module" } },
};

/**
 * The binding of a name `vitest` gives one of `names` (upstream's ImportDetails with source `vitest`): a named value
 * import, or a key of a destructured `require`/`await import`. A default value (`{ it = x }`) or rest element is not one.
 */
function vitestBinding(names) {
  return {
    any: [
      {
        resolves: {
          binding: {
            site: {
              kind: "import_specifier",
              has: { field: "name", regex: oneOf(names) },
              inside: {
                kind: "import_statement",
                stopBy: "end",
                // A type-only import (`import type { it }`, `import type * as v`, `import type D from`), not a
                // default import named `type` (`import type, { it } from`), which imports values.
                not: { regex: "^import\\s+type\\s*(?:[{*]|[\\w$]+\\s+from\\b)" },
                has: { field: "source", matches: "vitest-source" },
              },
            },
          },
        },
      },
      {
        resolves: {
          binding: {
            occurrence: {
              any: [
                { kind: "shorthand_property_identifier_pattern", regex: oneOf(names), inside: VITEST_DESTRUCTURING },
                {
                  kind: "identifier",
                  inside: { kind: "pair_pattern", field: "value", has: { field: "key", ...accessorKey(names) }, inside: VITEST_DESTRUCTURING },
                },
              ],
            },
          },
        },
      },
    ],
  };
}

/** The utilities for a set of function names: `vitest-<label>-head` and `vitest-<label>-chain`, and what they use. */
function functionUtilities(label, names) {
  const utils = {};
  const cases = names.filter(name => TEST_CASES.includes(name));
  const binding = [vitestBinding(names)];
  if (cases.length > 0) {
    const extended = cases.length === names.length ? label : `${label}-test`;
    Object.assign(utils, extendUtilities(extended, cases));
    binding.push({ matches: `vitest-${extended}-extended` });
  }
  utils[`vitest-${label}-head`] = {
    kind: "identifier",
    any: [{ resolves: { global: names } }, ...binding, { regex: oneOf(names), matches: "vitest-test-context" }],
  };
  utils[`vitest-${label}-chain`] = chain(`vitest-${label}-chain`, `vitest-${label}-head`);
  return utils;
}

/** `vitest-<label>-extended`: a name declared `const x = <root>.extend(...)`, its root `vitest` binding one of `cases`. */
function extendUtilities(label, cases) {
  return {
    [`vitest-${label}-extended`]: {
      resolves: {
        binding: {
          site: {
            kind: "variable_declarator",
            all: [{ has: { field: "name", kind: "identifier" } }, { has: { field: "value", matches: `vitest-${label}-extend-call` } }],
          },
        },
      },
    },
    [`vitest-${label}-extend-call`]: {
      any: [
        { kind: "call_expression", has: { field: "function", matches: `vitest-${label}-extend-member` } },
        { kind: "parenthesized_expression", has: { matches: `vitest-${label}-extend-call` } },
      ],
    },
    [`vitest-${label}-extend-member`]: {
      any: [
        {
          kind: "member_expression",
          all: [{ has: { field: "property", regex: "^extend$" } }, { has: { field: "object", matches: `vitest-${label}-root` } }],
        },
        { kind: "parenthesized_expression", has: { matches: `vitest-${label}-extend-member` } },
      ],
    },
    [`vitest-${label}-root`]: chain(`vitest-${label}-root`, `vitest-${label}-bound`),
    [`vitest-${label}-bound`]: { kind: "identifier", any: [vitestBinding(cases), { matches: `vitest-${label}-extended` }] },
  };
}

/** A node whose chain of member objects and call functions, through parentheses, starts at a node `head` matches. */
function chain(self, head) {
  return {
    any: [
      { matches: head },
      { kind: "member_expression", has: { field: "object", matches: self } },
      { kind: "call_expression", has: { field: "function", matches: self } },
      { kind: "parenthesized_expression", has: { matches: self } },
    ],
  };
}

/** The first property (not a rest element) of the first object-pattern parameter of a function upstream reads. */
const FIRST_CONTEXT_PROPERTY = {
  nthChild: { position: 1, ofRule: { any: [{ kind: "pair_pattern" }, { kind: "shorthand_property_identifier_pattern" }, { kind: "object_assignment_pattern" }] } },
  inside: {
    kind: "object_pattern",
    inside: {
      ...PARAMETER,
      nthChild: { position: 1, ofRule: { ...PARAMETER, has: { field: "pattern", kind: "object_pattern" } } },
      inside: { kind: "formal_parameters", inside: { any: [...FUNCTION_KINDS, { kind: "method_definition" }] } },
    },
  },
};

/**
 * The first property of a function's first object-pattern parameter, whose key is the name: `{ it }`, `{ it = x }`,
 * `{ it: it }` or `{ it: it = x }` (upstream compares that property's identifier key with the name).
 */
const CONTEXT_PROPERTY_NAME = {
  any: [
    { kind: "shorthand_property_identifier_pattern", any: [FIRST_CONTEXT_PROPERTY, { inside: { kind: "object_assignment_pattern", field: "left", ...FIRST_CONTEXT_PROPERTY } }] },
    {
      kind: "identifier",
      pattern: "$CONTEXT_NAME",
      any: [{ inside: { field: "value", matches: "vitest-context-pair" } }, { inside: { kind: "assignment_pattern", field: "left", inside: { field: "value", matches: "vitest-context-pair" } } }],
    },
  ],
};

/** Test-context names (upstream's `testContext`), which upstream classifies by their own name. */
const TEST_CONTEXT = {
  "vitest-context-pair": { kind: "pair_pattern", ...FIRST_CONTEXT_PROPERTY, has: { field: "key", kind: "property_identifier", pattern: "$CONTEXT_NAME" } },
  "vitest-test-callback": {
    any: FUNCTION_KINDS,
    all: [
      {
        any: [
          { has: { field: "parameter", kind: "identifier" } },
          { has: { field: "parameters", has: { ...PARAMETER, has: { field: "pattern", kind: "identifier" }, not: { has: { field: "value", regex: "[\\s\\S]" } } } } },
        ],
      },
      { inside: { kind: "arguments", inside: { kind: "call_expression", has: { field: "function", kind: "identifier", regex: oneOf(TEST_CASES) } } } },
    ],
  },
  "vitest-test-context": {
    any: [
      {
        resolves: {
          binding: {
            form: "parameter",
            site: {
              any: [
                { inside: { kind: "formal_parameters", inside: { matches: "vitest-test-callback" } } },
                { kind: "identifier", inside: { field: "parameter", matches: "vitest-test-callback" } },
              ],
            },
          },
        },
      },
      { resolves: { binding: { form: "parameter", occurrence: CONTEXT_PROPERTY_NAME } } },
      // A test callback's own name (`test('x', function it(ctx) { ... })`), which upstream also reads as a context.
      { resolves: { binding: { scope: "functionName", site: { matches: "vitest-test-callback" } } } },
    ],
  },
};

/**
 * `vitest-<modifier>-modifier`: a member chain whose first `.<modifier>` link (from the head) binds `$MODIFIER`, the
 * property upstream reports (`vitestFnCall.members.find(...)`), so a finding sits where the plugin's does.
 * `vitest-<modifier>-option`: a call's arguments whose second is an object whose last `<modifier>` property is
 * `<modifier>: true` (upstream's `findVitestModeProperty`, `test('x', { only: true }, fn)`); its key binds `$MODIFIER`.
 */
function modifierUtilities(modifier) {
  const self = `vitest-${modifier}-modifier`;
  // Upstream's candidates for `findLast`: every property (methods and accessors too, not spreads) whose key reads as
  // the modifier; only the last one counts, and only when it is `<modifier>: true`.
  const property = {
    any: [
      { kind: "pair", has: { field: "key", ...accessorKey([modifier]) } },
      { kind: "shorthand_property_identifier", regex: `^${modifier}$` },
      { kind: "method_definition", has: { field: "name", ...accessorKey([modifier]) } },
    ],
  };
  return {
    [self]: {
      kind: "member_expression",
      any: [{ has: { field: "object", matches: self } }, { has: { field: "property", regex: `^${modifier}$`, pattern: "$MODIFIER" } }],
    },
    [`vitest-${modifier}-option`]: {
      kind: "arguments",
      has: {
        nthChild: { position: 2, ofRule: { not: { kind: "comment" } } },
        kind: "object",
        has: {
          ...property,
          nthChild: { position: 1, reverse: true, ofRule: property },
          all: [
            { kind: "pair" },
            { has: { field: "value", kind: "true" } },
            // ESTree's key: inside computed brackets, the expression.
            {
              has: {
                field: "key",
                any: [
                  { kind: "computed_property_name", has: { any: [{ kind: "identifier" }, { kind: "string" }, { kind: "template_string" }], pattern: "$MODIFIER" } },
                  { not: { kind: "computed_property_name" }, pattern: "$MODIFIER" },
                ],
              },
            },
          ],
        },
      },
    },
  };
}

/** The define clause each document opens with: the utilities for the function sets and modifiers it names. */
function define(labels, modifiers = []) {
  const utils = {};
  for (const label of labels) Object.assign(utils, functionUtilities(label, FUNCTIONS[label]));
  for (const modifier of modifiers) Object.assign(utils, modifierUtilities(modifier));
  Object.assign(utils, TEST_CONTEXT, MODULE_SOURCE);
  return { id: "vitest-functions", define: { language: "typescript", utils } };
}

const violation = (rule, message, anchor = "witness.span") => ({ anchor, code: `vitest/${rule}`, message, suppress: { eslint: { rules: [`vitest/${rule}`] } } });
const ast = rule => ({ ast: { language: "typescript", rule } });

const DOCUMENTS = {
  "vitest-expect-expect": {
    mode: "steer",
    description: "A test whose function never calls `expect` or `assert`.",
    clauses: [
      define(["test"]),
      {
        id: "no-assertion",
        forbid: ast({
          kind: "call_expression",
          all: [
            {
              has: {
                field: "function",
                // A valid `it`/`test` chain of the modifiers this port reads, or the call `.each(...)` returns.
                any: [
                  { regex: validChain(["it", "test"], links => links.every(link => EXPECT_LINKS.includes(link))) },
                  {
                    kind: "call_expression",
                    has: { field: "function", regex: validChain(["it", "test"], links => links.at(-1) === "each" && links.slice(0, -1).every(link => EXPECT_LINKS.includes(link))) },
                  },
                ],
                all: [{ any: [{ matches: "vitest-test-head" }, { has: { field: "object", matches: "vitest-test-chain" } }, { has: { field: "function", matches: "vitest-test-chain" } }] }],
              },
            },
            {
              has: {
                field: "arguments",
                has: {
                  any: [{ kind: "arrow_function" }, { kind: "function_expression" }],
                  not: { has: { stopBy: "end", kind: "call_expression", has: { field: "function", regex: "^(?:expect|assert)\\b" } } },
                },
              },
            },
          ],
        }),
        violation: violation("expect-expect", "This test asserts nothing, so it passes whatever the code does. Add an expect."),
      },
    ],
  },
  "vitest-no-disabled-tests": {
    mode: "log",
    description: "A disabled test: `.skip`, `xit`, `xtest`, `xdescribe`, a test with no function, or `pending()`.",
    clauses: [
      define(["skippable", "xdisabled", "test", "block"], ["skip"]),
      {
        id: "skip",
        // Upstream reports the `.skip` property, a `{ skip: true }` option's key, or the call's head (`xit`, `pending`);
        // `$MODIFIER` is that node.
        forbid: ast({
          kind: "call_expression",
          any: [
            {
              has: {
                field: "function",
                any: [
                  {
                    kind: "member_expression",
                    regex: validChain(["describe", "it", "test", "suite", "bench"], including("skip")),
                    has: { field: "object", matches: "vitest-skippable-chain" },
                    matches: "vitest-skip-modifier",
                  },
                  {
                    kind: "identifier",
                    pattern: "$MODIFIER",
                    any: [
                      { regex: "^(?:xit|xtest|xdescribe)$", matches: "vitest-xdisabled-head" },
                      // Upstream reports `pending()` only where nothing binds `pending`.
                      { regex: "^pending$", resolves: { global: ["pending"] } },
                    ],
                  },
                  // `xit.each([])(...)` and `xit.each\`\`(...)`: the call `each` returns, reported at its head.
                  {
                    kind: "call_expression",
                    has: {
                      field: "function",
                      kind: "member_expression",
                      regex: "^(?:xit|xtest|xdescribe)\\.each$",
                      has: { field: "object", kind: "identifier", pattern: "$MODIFIER", matches: "vitest-xdisabled-head" },
                    },
                  },
                ],
              },
            },
            {
              all: [
                { has: { field: "function", regex: validChain(FUNCTIONS.block), matches: "vitest-block-chain" } },
                { has: { field: "arguments", matches: "vitest-skip-option" } },
              ],
            },
          ],
        }),
        violation: violation("no-disabled-tests", "This test is disabled. Fix it or delete it.", "capture.MODIFIER"),
      },
      {
        id: "missing-function",
        forbid: ast({
          kind: "call_expression",
          all: [
            { has: { field: "function", kind: "identifier", regex: "^(?:it|test)$", matches: "vitest-test-head" } },
            { has: { field: "arguments", kind: "arguments", not: { has: { nthChild: 2 } } } },
          ],
        }),
        violation: violation("no-disabled-tests", "This test has no function, so it never runs."),
      },
    ],
  },
  "vitest-no-focused-tests": {
    mode: "steer",
    description: "A focused test (`.only`), which makes Vitest skip every other test in the file.",
    clauses: [
      define(["focusable"], ["only"]),
      {
        id: "only",
        // Upstream reports the `.only` property, or a `{ only: true }` option's key; `$MODIFIER` is that node.
        forbid: ast({
          kind: "call_expression",
          any: [
            {
              has: {
                field: "function",
                kind: "member_expression",
                regex: validChain(["describe", "it", "test", "suite", "bench"], including("only")),
                has: { field: "object", matches: "vitest-focusable-chain" },
                matches: "vitest-only-modifier",
              },
            },
            {
              all: [
                { has: { field: "function", regex: validChain(FUNCTIONS.focusable), matches: "vitest-focusable-chain" } },
                { has: { field: "arguments", matches: "vitest-only-option" } },
              ],
            },
          ],
        }),
        violation: violation("no-focused-tests", "Remove .only: a focused test makes Vitest skip every other test.", "capture.MODIFIER"),
      },
    ],
  },
};

// A rule's id is its document's file name (docs/pack-format.md, the pack contract's check 1).
for (const [id, { mode, description, clauses }] of Object.entries(DOCUMENTS)) {
  const document = { format: "rerail.policy/v1", id, revision: 2, mode, description, scope: SCOPE, clauses };
  writeFileSync(join(RULES, `${id}.json`), `${JSON.stringify(document, null, 2)}\n`);
}
