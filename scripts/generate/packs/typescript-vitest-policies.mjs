// Writes the TypeScript pack's @vitest/eslint-plugin 1.6.27 candidates: packs/typescript/candidates/rules/vitest-*.json
// (listed in scripts/generate/README.md; edit this script, not the JSON). It spells once the checks the three ports
// share (which calls are Vitest's, upstream's valid chains, where findings sit) and writes each document the utilities
// it needs. It reads no tools; run offline by a maintainer, never in CI; the output is deterministic.
//
// Usage (from the repository root): node scripts/generate/packs/typescript-vitest-policies.mjs
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

const RULES = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..", "packs", "typescript", "candidates", "rules");

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

/** Spaces, line breaks and comments, which upstream's accessors never see. */
const TRIVIA = "(?:\\s|/\\*[^*]*\\*+(?:[^/*][^*]*\\*+)*/|//[^\\n]*)*";
/** A member access between links (upstream reads the chain's accessors, not its text). */
const SEP = `${TRIVIA}\\.${TRIVIA}`;
/** A head: any identifier, since an import or `.extend` may rename the function (`import { test as check }`). */
const HEAD = "[\\w$]+";

/** An alternation matching exactly the given link lists as `.a.b` text, factored by common prefixes. */
function linksPattern(chains) {
  const tree = new Map();
  let end = false;
  for (const [first, ...rest] of chains) {
    if (first === undefined) end = true;
    else tree.set(first, [...(tree.get(first) ?? []), rest]);
  }
  const branches = [...tree].sort(([a], [b]) => (a < b ? -1 : 1)).map(([link, rests]) => `${link}${linksPattern(rests)}`);
  if (branches.length === 0) return "";
  return `(?:${SEP}(?:${branches.join("|")}))${end ? "?" : ""}`;
}

/**
 * The links a callee may carry after its head: `keep` over a valid chain's links; `free`, the pattern for a head whose
 * chain upstream does not check (a test context, `vi`, `vitest`); and `expect`/`expectTypeOf`, the pattern after that
 * function, whose links upstream reads as modifiers before a matcher (absent where a spec leaves it out).
 */
const ANY_LINKS = { keep: () => true, free: `(?:${SEP}${HEAD})*` };
const NO_LINKS = { keep: links => links.length === 0, free: "" };

/** Chai's chainable properties upstream reads as language chains before an `expect` matcher (parseExpectCallExpression). */
const CHAI_CHAINS = [
  ...["also", "and", "at", "be", "been", "but", "does", "has", "have", "is", "itself", "of", "same", "still", "that", "to", "which", "with"],
  ...["all", "any", "deep", "nested", "ordered", "own"],
  ...["a", "an", "contain", "contains", "include", "includes", "length", "lengthOf", "respondTo"],
];
/** expectTypeOf's modifiers besides `not` (expectTypeOfModifiers). */
const EXPECT_TYPE_OF_MODIFIERS = ["resolves", "returns", "branded", "asserts", "constructorParameters", "parameters", "thisParameter", "guards", "instance", "items"];

/**
 * `expect.<links>.<matcher>`: language chains anywhere, at most one `not` and one of `resolves`/`rejects`
 * (hasInvalidExpectChain), an uncalled `assert` first. `expectTypeOf.<links>.<matcher>`: its modifiers, one `not`.
 */
function expectPatterns(matcher) {
  const chains = `(?:${SEP}(?:${CHAI_CHAINS.join("|")}))*`;
  const promise = `${SEP}(?:resolves|rejects)`;
  const not = `${SEP}not`;
  const orders = ["", `${not}${chains}`, `${promise}${chains}`, `${not}${chains}${promise}${chains}`, `${promise}${chains}${not}${chains}`];
  const modifiers = `(?:${SEP}(?:${EXPECT_TYPE_OF_MODIFIERS.join("|")}))*`;
  return {
    expect: `(?:${SEP}assert)?${chains}(?:${orders.join("|")})${SEP}${matcher}`,
    expectTypeOf: `${modifiers}(?:${not}${modifiers})?${SEP}${matcher}`,
  };
}

const including = member => ({
  keep: links => links.includes(member),
  free: `(?:${SEP}${HEAD})*${SEP}${member}(?:${SEP}${HEAD})*`,
  // `expect.skip()`, `expect.not.skip()`: for `expect`, the member is the matcher.
  ...expectPatterns(member),
});
const among = allowed => ({ keep: links => links.every(link => allowed.includes(link)), free: `(?:${SEP}(?:${allowed.join("|")}))*` });
/** A table call (`.each`/`.for`) the test is made by: `it.each([...])('x', fn)`, `it.only.for([...])('x', fn)`. */
const tableAfter = allowed => ({
  keep: links => ["each", "for"].includes(links.at(-1)) && links.slice(0, -1).every(link => allowed.includes(link)),
  free: `(?:${SEP}(?:${allowed.join("|")}))*${SEP}(?:each|for)`,
});
/** Functions whose chains upstream does not check against ValidVitestFnCallChains. */
const UNCHECKED = ["vi", "vitest"];
const EXPECTS = ["expect", "expectTypeOf"];
/** The modifiers the expect-expect port reads on a test (upstream skips `.skip`, `.todo`, `.extend` and `.scoped`). */
const EXPECT_LINKS = ["only", "concurrent", "sequential", "fails"];
const labelOf = names => [...names].sort().join("-");
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
            // The declarator first: `occurrence` reads every reference of the name, so it runs only for a name a
            // `require('vitest')` or `await import('vitest')` declarator binds.
            site: { kind: "variable_declarator", has: { field: "value", matches: "vitest-module" } },
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

/**
 * A callee (a call's `function`) that names a Vitest function in `names` by a chain upstream accepts for it, its links
 * passing `links`. The head's binding decides the function (an import's original name, a `.extend`'s root), so each
 * grammar's chains are checked against heads bound to that grammar's functions, whatever their local name; a test
 * context and the unchecked functions take `links.free`. Registers the utilities it names in `utils`.
 */
function vitestCallee(utils, names, links) {
  const any = [];
  for (const grammar of Object.values(CHAIN_GRAMMAR)) {
    const heads = grammar.heads.filter(head => names.includes(head));
    const chains = chainLinks(grammar).filter(links.keep);
    if (heads.length > 0 && chains.length > 0) any.push({ regex: `^${HEAD}${linksPattern(chains)}$`, matches: namedCallee(utils, heads) });
  }
  const unchecked = names.filter(name => UNCHECKED.includes(name));
  if (unchecked.length > 0) any.push({ regex: `^${HEAD}${links.free}$`, matches: namedCallee(utils, unchecked) });
  for (const name of names.filter(name => EXPECTS.includes(name))) {
    if (links[name] !== undefined) any.push({ regex: `^${HEAD}${links[name]}$`, matches: namedCallee(utils, [name]) });
  }
  any.push({ regex: `^${HEAD}${links.free}$`, matches: contextCallee(utils, names) });
  return { any };
}

/** `vitest-<label>-callee`: a chain whose head is a global or a `vitest` binding of one of `names`. */
function namedCallee(utils, names) {
  const label = labelOf(names);
  const cases = names.filter(name => TEST_CASES.includes(name));
  const any = [{ resolves: { global: names } }, vitestBinding(names)];
  if (cases.length > 0) {
    Object.assign(utils, extendUtilities(labelOf(cases), cases));
    any.push({ matches: `vitest-${labelOf(cases)}-extended` });
  }
  utils[`vitest-${label}-named`] = { kind: "identifier", any };
  utils[`vitest-${label}-callee`] = chain(`vitest-${label}-callee`, `vitest-${label}-named`);
  return `vitest-${label}-callee`;
}

/** `vitest-<label>-context-callee`: a chain whose head is a test context named one of `names`. */
function contextCallee(utils, names) {
  const label = labelOf(names);
  utils[`vitest-${label}-context`] = { kind: "identifier", regex: oneOf(names), matches: "vitest-test-context" };
  utils[`vitest-${label}-context-callee`] = chain(`vitest-${label}-context-callee`, `vitest-${label}-context`);
  return `vitest-${label}-context-callee`;
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

/** A document's clauses: a define clause with every utility its clauses named, then the clauses `build` returns. */
function withDefine(build, modifiers = []) {
  const utils = {};
  const clauses = build(utils);
  for (const modifier of modifiers) Object.assign(utils, modifierUtilities(modifier));
  Object.assign(utils, TEST_CONTEXT, MODULE_SOURCE);
  return [{ id: "vitest-functions", define: { language: "typescript", utils } }, ...clauses];
}

const violation = (rule, message, anchor = "witness.span") => ({ anchor, code: `vitest/${rule}`, message, suppress: { eslint: { rules: [`vitest/${rule}`] } } });
const ast = rule => ({ ast: { language: "typescript", rule } });

/**
 * An assertion call as expect-expect's default `assertFunctionNames` read it (`expect`, `assert`, case-insensitive): a
 * call whose name (getNodeName: the chain through members and calls, parentheses aside) starts with one of them. A
 * tagged template (`` expect`x` ``) is no call upstream, though the grammar reads it as one.
 */
function assertionCall(utils) {
  utils["vitest-assertion-name"] = { kind: "identifier", regex: "(?i)^(?:expect|assert)$" };
  utils["vitest-assertion-callee"] = chain("vitest-assertion-callee", "vitest-assertion-name");
  return { kind: "call_expression", has: { field: "function", matches: "vitest-assertion-callee" }, not: { has: { field: "arguments", kind: "template_string" } } };
}

const DOCUMENTS = {
  "vitest-expect-expect": {
    mode: "steer",
    description: "A test whose function never calls `expect` or `assert`.",
    clauses: withDefine(utils => [
      {
        id: "no-assertion",
        forbid: ast({
          kind: "call_expression",
          all: [
            {
              has: {
                field: "function",
                // A test called with the modifiers this port reads, or the call `.each(...)`/`.for(...)` returns;
                // upstream passes over a call of an identifier named `bench`.
                not: { kind: "identifier", regex: "^bench$" },
                any: [
                  vitestCallee(utils, FUNCTIONS.test, among(EXPECT_LINKS)),
                  { kind: "call_expression", has: { field: "function", ...vitestCallee(utils, FUNCTIONS.test, tableAfter(EXPECT_LINKS)) } },
                ],
              },
            },
            // Upstream marks a test checked when an assertion call sits anywhere among its arguments, or inside a
            // function declaration an argument names (`it('x', helper)`); anything else, a test with no callback
            // included, asserts nothing.
            {
              has: {
                field: "arguments",
                not: {
                  any: [
                    { has: { stopBy: "end", ...assertionCall(utils) } },
                    {
                      has: {
                        kind: "identifier",
                        resolves: {
                          binding: {
                            site: {
                              kind: "function_declaration",
                              has: { stopBy: "end", ...assertionCall(utils) },
                            },
                          },
                        },
                      },
                    },
                  ],
                },
              },
            },
          ],
        }),
        violation: violation("expect-expect", "This test asserts nothing, so it passes whatever the code does. Add an expect."),
      },
    ]),
  },
  "vitest-no-disabled-tests": {
    mode: "log",
    description: "A disabled test: `.skip`, `xit`, `xtest`, `xdescribe`, a test with no function, or `pending()`.",
    clauses: withDefine(
      utils => [
        {
          id: "skip",
          // Upstream reports the `.skip` property, a `{ skip: true }` option's key, or the call's head (`xit`,
          // `pending`); `$MODIFIER` is that node.
          forbid: ast({
            kind: "call_expression",
            any: [
              {
                has: {
                  field: "function",
                  any: [
                    { kind: "member_expression", ...vitestCallee(utils, FUNCTIONS.skippable, including("skip")), matches: "vitest-skip-modifier" },
                    {
                      kind: "identifier",
                      pattern: "$MODIFIER",
                      any: [
                        vitestCallee(utils, FUNCTIONS.xdisabled, NO_LINKS),
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
                        ...vitestCallee(utils, FUNCTIONS.xdisabled, { keep: links => links.join(".") === "each", free: `${SEP}each` }),
                        has: { field: "object", kind: "identifier", pattern: "$MODIFIER" },
                      },
                    },
                  ],
                },
              },
              {
                all: [
                  { has: { field: "function", ...vitestCallee(utils, FUNCTIONS.block, ANY_LINKS) } },
                  { has: { field: "arguments", matches: "vitest-skip-option" } },
                ],
              },
            ],
          }),
          violation: violation("no-disabled-tests", "This test is disabled. Fix it or delete it.", "capture.MODIFIER"),
        },
        {
          id: "missing-function",
          // A test (whose links, if any, are all `.skip`) called with fewer than two arguments.
          forbid: ast({
            kind: "call_expression",
            all: [
              { has: { field: "function", ...vitestCallee(utils, FUNCTIONS.test, { keep: links => links.every(link => link === "skip"), free: `(?:${SEP}skip)*` }) } },
              { has: { field: "arguments", kind: "arguments", not: { has: { nthChild: 2 } } } },
            ],
          }),
          violation: violation("no-disabled-tests", "This test has no function, so it never runs."),
        },
      ],
      ["skip"],
    ),
  },
  "vitest-no-focused-tests": {
    mode: "steer",
    description: "A focused test (`.only`), which makes Vitest skip every other test in the file.",
    clauses: withDefine(
      utils => [
        {
          id: "only",
          // Upstream reports the `.only` property, or a `{ only: true }` option's key; `$MODIFIER` is that node.
          forbid: ast({
            kind: "call_expression",
            any: [
              { has: { field: "function", kind: "member_expression", ...vitestCallee(utils, FUNCTIONS.focusable, including("only")), matches: "vitest-only-modifier" } },
              {
                all: [
                  { has: { field: "function", ...vitestCallee(utils, FUNCTIONS.focusable, ANY_LINKS) } },
                  { has: { field: "arguments", matches: "vitest-only-option" } },
                ],
              },
            ],
          }),
          violation: violation("no-focused-tests", "Remove .only: a focused test makes Vitest skip every other test.", "capture.MODIFIER"),
        },
      ],
      ["only"],
    ),
  },
};

// A rule's id is its document's file name (docs/pack-format.md, the pack contract's check 1).
for (const [id, { mode, description, clauses }] of Object.entries(DOCUMENTS)) {
  const document = { format: "rerail.policy/v1", id, revision: 2, mode, description, scope: SCOPE, clauses };
  writeFileSync(join(RULES, `${id}.json`), `${JSON.stringify(document, null, 2)}\n`);
}
