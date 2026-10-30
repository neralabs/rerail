# Rerail rule pack: TypeScript, candidates

The ports in `rules/` here are still being evaluated: scored against the tools' expected findings and reported, never
gated and never shipped. This file is not published; the pack's [README](../README.md) covers the rules it ships.

Every typescript-eslint 8.70.1 rule that needs no type information has a port, shipped or a candidate here, except
`no-restricted-types` (the pack README says why).

## `init-declarations` can't match upstream's tests

typescript-eslint declares `'always'` as `init-declarations`' default. Its wrapper around ESLint's core rule has no
`meta.defaultOptions`, so with no options ESLint 10 passes the core rule no mode, and it reports nothing. Every upstream
test that expects a finding sets the option, so upstream's own tests never notice. The generator lints each case at the
rule's default options and leaves out the cases that set options, so the expected findings hold no upstream finding of
the rule at all, whatever its document says. The candidate ports the declared default, `'always'`. With that option
set, it agrees with typescript-eslint on every upstream test case, fixture, rename variant and sample.

## `prefer-optional-chain` needs type information

typescript-eslint 8.70.1 marks it `requiresTypeChecking` and calls `getParserServices` with no fallback, and every one
of its upstream tests runs with a TypeScript project. The expected findings are made without type information, so the
generator keeps none of its cases and it is not listed in `conformance.json`.

## The vitest ports fail the in-scope check

@vitest/eslint-plugin's approved `recommended` configuration sets no `files`, so ESLint lints its default `.js`, `.mjs`
and `.cjs` files (`upstream/eslint-plugin-vitest/gates.yaml`, `upstream.lints`). The three `vitest-*` ports run on
TypeScript test files only, as every TypeScript-pack rule runs on TypeScript files, so `loads`' in-scope check fails for
each. They pass every other check. Shipping them needs either the ports on JavaScript files too or a gate that accepts
an approved configuration linting TypeScript test files.

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `typescript-eslint-init-declarations`

In `always` mode, the default the rule declares. typescript-eslint 8.70.1 does not hand that default on to ESLint's core rule, so configured without options it reports nothing; this document is the rule as `["error", "always"]` runs it. It reports each variable declared by a plain name without an initial value, at the name. The head of a `for` loop counts as initialized; a declaration that is itself the body of a `for`, `for...in` or `for...of` loop counts as uninitialized even with a value, and is then reported over the whole declarator. Declarations marked `declare`, and everything inside a `declare namespace`, `declare module` or `declare global` block, are skipped. The ancestor walk (`outside-declared-module`) recognizes a declared module by its leading `declare` keyword using positive tests only, so the document is still evaluated in files with syntax errors. As in TypeScript, comments after `declare` are skipped, and a line break after it (bare or inside a block comment) makes `declare` an ordinary name rather than a modifier, so what follows is checked as a plain statement.

### `vitest-no-disabled-tests`

Disabled forms: `.skip`, `xit`, `xtest`, `xdescribe`, a test with no function, `pending()`.

### `vitest-no-focused-tests`

Divergence (`not-comparable`): a jest sample whose `// eslint-disable-next-line jest/no-focused-tests` names another plugin's rule. ESLint reports the `test.only` below it; the port's `suppress.oxlint` matches the directive by its part after the last `/`, as Oxlint does, so the comment silences it.
