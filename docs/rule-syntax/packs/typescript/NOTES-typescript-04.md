# typescript-04: prefer-for-of, unified-signatures, vitest no-disabled-tests, no-focused-tests

Ground truth: ESLint 9.39.5 with typescript-eslint 8.70.1 and @vitest/eslint-plugin 1.6.27 (the versions `pack.json`
covers; npx, on a box). prefer-for-of, unified-signatures and no-focused-tests are `same` as their JSON originals on
every corpus (examples, fixtures, rerail-typescript). no-disabled-tests is `different`: 5 translation-only findings on
rerail-typescript, each one ESLint reports and the original misses (below). On the whole rerail-typescript corpus
ESLint with the four rules reports 5 findings; the translations report the same 5 keys.

## Review fixes (typescript-04.review.md)

The independent review found 21 defects, each with a counterexample. All 21 were confirmed against ESLint on a box
and are fixed; none is disputed. Each counterexample is now a `.test` in its rule (`rerail-engine test-rr`: 22 tests,
all pass), and each test's expected keys were checked to be exactly ESLint's on the test's fixture files.

### typescript-eslint-prefer-for-of

1. `a[length]`: the rule tests the property's name, not `computed`. `length_of` takes `o[length]` too.
2. `?.` anywhere below the length: the rule only rejects a ChainExpression, i.e. a chain running up to `.length`.
   `in_chain` follows members' objects, calls' functions and `!`, never parentheses: `(ob?.b).length` and
   `f(ob?.b).length` count, `ob?.b.length`, `ob?.b!.length`, `g?.().length` do not (all checked against ESLint).
3. `var i = 0` or `for (var i in/of …)` in the body: eslint-scope's write reference to the index, whose parent is no
   MemberExpression. A body `var` declaring the index's variable with an initialiser (`initialised`) is another use.
   `var i;` (no reference) and a nested function's `var i = 0` (another variable) still report, as in ESLint.
4. `0n`: `value` gives the number 0 for `0n`; `literal` now refuses a BigInt (text ending in `n`), also for `+= 1n`.

### typescript-eslint-unified-signatures

1. `[sa]`/`[sb]`: a computed Identifier key is `identifier_<name>` (computed flag `0`), so `[sa]` and `[sb]` are two
   groups and `[(sa)]` joins `[sa]`; other computed expressions stay `undefined`, other literals (`true`, `null`, a
   regex) key by raw text.
2. `declare global { … }`: tree-sitter's `ambient_declaration` holding a `statement_block` is a scope.
3. `export default class { … }`: tree-sitter's anonymous `class` under an export is a scope.
4. `{ [K in "a"]: T }`: typeContainsTypeParameter follows a mapped type's `typeAnnotation`; `reaches` does too
   (`type_annotation` and the `-?:`/`+?:`/`?:` forms).
5. Multi-line JSDoc: `match("/\\*.*", c)` gave no row for a comment spanning lines; `prefix("/*", text)` does.
   The options are now read through `on(x)` (`option(x, "true")`) so a test can set one with `.given on`: a test
   cannot declare `option`, a core relation.

### vitest-no-disabled-tests, vitest-no-focused-tests

The twelve defects were all in how a call is taken for a Vitest call. `tsx/typescript-04` now follows
`parseVitestFnCall` itself instead of `tsx/ts-b-vitest`'s `vitest_call` (expect chains aside, which no-disabled-tests
still takes from there):

- The head as `resolveScope` resolves it: a test context is any name declared by a function expression, arrow
  function or method whose first object-pattern parameter's first property is keyed by the name (any function:
  no-focused 1, 3), or by such a function with a plain-named parameter passed to a call of an identifier named it,
  test, fit, xit, xtest or bench (any parameter name: no-disabled 1). A context's name is its local name and its
  chain is never checked. Imports, `require`, `await import` and `.extend` give the imported name.
- Chain validity as `ValidVitestFnCallChains`: its 1386 chains are exactly this grammar (checked by generating it
  and comparing sets): it/test: an optional condition (skipIf, runIf) first, distinct modifiers (skip, only,
  concurrent, sequential, todo, fails), an optional table (each, for) last, three members at most, or `extend` or
  `scoped` alone; describe/suite the same with shuffle for fails; bench a condition and skip, only, todo; xit, xtest,
  xdescribe nothing or `each`; fit, fdescribe nothing. vi and vitest unchecked. (no-disabled 3, 4; no-focused 4.)
- The call's place: not the callee or an argument of a call, not the object or index of a member (no-focused 5);
  a chain's head not itself called; `each` last only on a call or tagged template, a tagged template only with
  `each` last.
- no-disabled-tests: the first `skip` member, else (describe or test types) the last `skip`-keyed property if it is
  `skip: true`, else the head if the name starts with `x` (xit's `skip: true`: at the key, no-disabled 5); missing
  function only for test types with fewer than two arguments and no member but `skip` (no-disabled 2);
  `(pending)()` (no-disabled 6).
- no-focused-tests: the name it, test or describe; the first `only` member at any index (no-focused 2), else the
  options' last `only`-keyed property if `only: true`.
- The mode option: the second argument with parentheses dropped, keys `x`, `"x"`, `[x]` (an Identifier, whatever
  its value), `["x"]`, value `true` through parentheses (no-focused 6, and the same for `skip`).

The `rerail-typescript` corpus (602 files) has 5 `context.skip()` calls in test callbacks: `test/engine/no-follow.test.ts`
35 and 53, `test/daemon/daemon.test.ts` 798, `test/api/api-mcp-caller.test.ts` 141 and
`test/engine/catalog/checkouts/checkout-spelling.test.ts` 43. ESLint reports all five (`disabledTest` at `skip`); the
JSON original reports none (its context root is only a name `it`), the translation reports all five. The review
named three; the corpus now has five.

Not followed (no corpus has them; noted, not tested): a chain property that is itself a member or call
(`it[a.b]`, joined into the chain by getNodeChain), a string-literal head, a call inside an optional chain whose
ESTree parent is the ChainExpression (`f(it?.only())`), a name with several definitions (the plugin takes the last),
`import { type it } from "vitest"` (the plugin ignores a specifier's `type`), and `vitestImports`/`globalAliases`
settings.

## Engine gap: `earlier` has no rows under run-policies

`earlier` (and `visit_index`, core.rerail `.input`) yield no rows for TypeScript under `run-policies`, as react-04
found. These rules order siblings by `precedes` instead. Until the engine gives `earlier` rows, a rule ordering
siblings should use `precedes`.

## Before the review

- unified-signatures reported nothing at first (`pair` ordered by `earlier`); `member_of` and `precedes` fixed it.
- no-focused-tests and no-disabled-tests each had one translation-only finding on
  `no-focused-tests/rerail-mode-option-overridden.test.ts` (`{ only: true, only() {} }`: the method is the last
  `only`), from `tsx/ts-b-vitest`'s `later_entry` using `earlier`. `mode_key` finds the last property by `precedes`.
- prefer-for-of had one translation-only finding, `prefer-for-of/044.ts:2` (`({ foo: arr[i] }) = { foo: 0 }`):
  `isAssignee` counts a Property value of an ObjectExpression that is itself an assignee; `place`/`written_by` do too.

Library change proposed: `tsx/ts-b-vitest`'s `vitest_call` has the same head, chain and place defects for
expect-expect; `tsx/typescript-04`'s `vitest_fn` could replace its test and describe part.
